//! Modal dialogs.

use std::path::PathBuf;

use egui::{Key, RichText};

use crate::app::Settings;
use crate::git::diff::Selection;
use crate::git::{FileChange, InheritedConfig, PullMode, RepoConfig, SshKey};
use crate::repo::{PatchAction, RepoTab};
use crate::ui::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

pub enum Dialog {
    CreateBranch {
        start: String,
        start_label: String,
        name: String,
        checkout: bool,
    },
    RenameBranch {
        old: String,
        name: String,
    },
    CreateTag {
        target: String,
        name: String,
        message: String,
        push: bool,
    },
    Pull {
        remote: String,
        branch: String,
        rebase: bool,
        autostash: bool,
    },
    Push {
        branch: String,
        remote: String,
        remote_branch: String,
        set_upstream: bool,
        force: bool,
        tags: bool,
    },
    Stash {
        message: String,
        untracked: bool,
        keep_index: bool,
    },
    Reset {
        target: String,
        mode: ResetMode,
    },
    Merge {
        source: String,
        no_ff: bool,
        squash: bool,
    },
    AddRemote {
        name: String,
        url: String,
    },
    EditRemote {
        old_name: String,
        old_url: String,
        name: String,
        url: String,
    },
    Discard {
        files: Vec<FileChange>,
    },
    DiscardPatch(Selection),
    /// Runs `commands` in sequence after confirmation.
    Confirm {
        title: String,
        text: String,
        button: String,
        destructive: bool,
        label: String,
        commands: Vec<Vec<String>>,
    },
    Clone {
        url: String,
        parent: String,
        name: String,
    },
    Settings(Settings),
    /// Add, rename, remove and reorder workspaces.
    Workspaces(Vec<crate::workspace::Edit>),
    RepoSettings {
        repo: String,
        original: RepoConfig,
        edit: RepoConfig,
        inherited: InheritedConfig,
        access: RepoAccess,
    },
}

/// What the Repository Settings dialog needs to offer account and key choices.
pub struct RepoAccess {
    /// Signed-in GitHub accounts.
    accounts: Vec<String>,
    /// Account Gitr picked automatically for this repository, once known.
    auto_account: Option<String>,
    /// Private keys found in `~/.ssh`.
    ssh_keys: Vec<String>,
    has_github_https: bool,
    has_ssh: bool,
}

/// `~/…` form of a path under the home directory, for display.
fn short_path(path: &str) -> String {
    match std::env::home_dir() {
        Some(home) => match path.strip_prefix(&*home.to_string_lossy()) {
            Some(rest) => format!("~{rest}"),
            None => path.to_owned(),
        },
        None => path.to_owned(),
    }
}

pub enum AppAction {
    Clone { url: String, dest: PathBuf },
    SaveWorkspaces(Vec<crate::workspace::Edit>),
    SaveSettings(Settings),
}

pub enum Outcome {
    Open,
    Close,
    App(AppAction),
}

const MULTILINE_FOCUS: &str = "gitr_dialog_multiline_focus";

fn s(v: &str) -> String {
    v.to_owned()
}

impl Dialog {
    pub fn confirm(
        title: impl Into<String>,
        text: impl Into<String>,
        button: impl Into<String>,
        label: impl Into<String>,
        commands: Vec<Vec<String>>,
    ) -> Dialog {
        Dialog::Confirm {
            title: title.into(),
            text: text.into(),
            button: button.into(),
            destructive: true,
            label: label.into(),
            commands,
        }
    }

    pub fn workspaces(workspaces: &[crate::workspace::Workspace]) -> Dialog {
        Dialog::Workspaces(
            workspaces
                .iter()
                .enumerate()
                .map(|(i, w)| (Some(i), w.name.clone()))
                .collect(),
        )
    }

    pub fn repo_settings(tab: &RepoTab) -> Dialog {
        let (config, inherited) = crate::git::load_repo_config(&tab.path);
        let urls: Vec<&str> = tab.refs.remotes.iter().map(|r| r.url.as_str()).collect();
        let access = RepoAccess {
            accounts: crate::github::accounts(),
            auto_account: urls
                .iter()
                .find_map(|u| crate::github::cached_account_for_url(u)),
            ssh_keys: crate::git::ssh_keys(),
            has_github_https: urls
                .iter()
                .any(|u| crate::github::github_https_repo(u).is_some()),
            has_ssh: urls
                .iter()
                .any(|u| u.starts_with("ssh://") || (!u.contains("://") && u.contains('@'))),
        };
        Dialog::RepoSettings {
            repo: tab.name.clone(),
            original: config.clone(),
            edit: config,
            inherited,
            access,
        }
    }

    pub fn pull(tab: &RepoTab) -> Dialog {
        let head = tab.refs.head_upstream();
        let (remote, branch) = head
            .and_then(|b| b.upstream.as_deref())
            .and_then(|u| u.split_once('/'))
            .map(|(r, b)| (r.to_owned(), b.to_owned()))
            .unwrap_or_else(|| {
                (
                    tab.default_remote().unwrap_or_default(),
                    tab.refs.head_branch.clone().unwrap_or_default(),
                )
            });
        let rebase =
            crate::git::config_value(&tab.path, "pull.rebase").is_some_and(|v| v == "true");
        Dialog::Pull {
            remote,
            branch,
            rebase,
            autostash: false,
        }
    }

    pub fn push(tab: &RepoTab) -> Dialog {
        let branch = tab.refs.head_branch.clone().unwrap_or_default();
        let upstream = tab
            .refs
            .local_branch(&branch)
            .and_then(|b| b.upstream.clone());
        let (remote, remote_branch) = upstream
            .as_deref()
            .and_then(|u| u.split_once('/'))
            .map(|(r, b)| (r.to_owned(), b.to_owned()))
            .unwrap_or_else(|| (tab.default_remote().unwrap_or_default(), branch.clone()));
        Dialog::Push {
            branch,
            remote,
            remote_branch,
            set_upstream: upstream.is_none(),
            force: false,
            tags: false,
        }
    }
}

fn buttons(ui: &mut egui::Ui, confirm: &str, enabled: bool, destructive: bool) -> (bool, bool) {
    let mut ok = false;
    let mut cancel = false;
    ui.add_space(8.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let mut text = RichText::new(confirm);
        let mut button = egui::Button::new(text.clone());
        if enabled {
            let fill = if destructive {
                theme::pal(ui).removed
            } else {
                theme::pal(ui).selection
            };
            text = text.color(egui::Color32::WHITE);
            button = egui::Button::new(text).fill(fill);
        }
        if ui.add_enabled(enabled, button).clicked() {
            ok = true;
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    // Plain Enter confirms unless a multi-line field is being edited; Cmd/Ctrl+Enter always does.
    let multiline_focused = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new(MULTILINE_FOCUS)))
        .unwrap_or(false);
    let enter = ui.input(|i| {
        i.key_pressed(Key::Enter)
            && !i.modifiers.shift
            && (i.modifiers.command || !multiline_focused)
    });
    (ok || (enabled && enter), cancel)
}

fn remote_combo(ui: &mut egui::Ui, id: &str, remote: &mut String, remotes: &[String]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(remote.as_str())
        .width(260.0)
        .show_ui(ui, |ui| {
            for r in remotes {
                ui.selectable_value(remote, r.clone(), r);
            }
        });
}

fn grid(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, add);
}

fn text_field(ui: &mut egui::Ui, value: &mut String, hint: &str, focus: bool) {
    let r = ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(260.0),
    );
    if focus && !r.has_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
}

pub fn show(ctx: &egui::Context, dialog: &mut Dialog, tab: Option<&mut RepoTab>) -> Outcome {
    let mut outcome = Outcome::Open;
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(MULTILINE_FOCUS), false));
    let modal = egui::Modal::new(egui::Id::new("gitr_dialog")).show(ctx, |ui| {
        ui.set_min_width(420.0);
        ui.set_max_width(480.0);
        outcome = body(ui, dialog, tab);
    });
    if modal.should_close() && matches!(outcome, Outcome::Open) {
        outcome = Outcome::Close;
    }
    outcome
}

fn body(ui: &mut egui::Ui, dialog: &mut Dialog, tab: Option<&mut RepoTab>) -> Outcome {
    let remotes = tab
        .as_ref()
        .map(|t| t.refs.remote_names())
        .unwrap_or_default();
    let close;
    let mut app = None;
    match dialog {
        Dialog::CreateBranch {
            start,
            start_label,
            name,
            checkout,
        } => {
            ui.heading("Create Branch");
            ui.add_space(6.0);
            grid(ui, "cb", |ui| {
                ui.label("Starting point:");
                ui.label(RichText::new(start_label.as_str()).monospace());
                ui.end_row();
                ui.label("Branch name:");
                text_field(ui, name, "feature/my-branch", true);
                ui.end_row();
                ui.label("");
                ui.checkbox(checkout, "Check out after create");
                ui.end_row();
            });
            let valid = !name.trim().is_empty() && !name.contains(' ');
            let (ok, cancel) = buttons(ui, "Create Branch", valid, false);
            if ok {
                if let Some(tab) = tab {
                    let name = name.trim().to_owned();
                    let args = if *checkout {
                        vec![s("checkout"), s("-b"), name.clone(), start.clone()]
                    } else {
                        vec![s("branch"), name.clone(), start.clone()]
                    };
                    tab.git(format!("Create branch {name}"), args);
                }
            }
            close = ok || cancel;
        }
        Dialog::RenameBranch { old, name } => {
            ui.heading("Rename Branch");
            ui.add_space(6.0);
            grid(ui, "rb", |ui| {
                ui.label("Branch:");
                ui.label(RichText::new(old.as_str()).monospace());
                ui.end_row();
                ui.label("New name:");
                text_field(ui, name, "", true);
                ui.end_row();
            });
            let valid = !name.trim().is_empty() && name.trim() != old;
            let (ok, cancel) = buttons(ui, "Rename", valid, false);
            if ok {
                if let Some(tab) = tab {
                    tab.git(
                        format!("Rename branch {old}"),
                        vec![s("branch"), s("-m"), old.clone(), name.trim().to_owned()],
                    );
                }
            }
            close = ok || cancel;
        }
        Dialog::CreateTag {
            target,
            name,
            message,
            push,
        } => {
            ui.heading("Create Tag");
            ui.add_space(6.0);
            grid(ui, "ct", |ui| {
                ui.label("Commit:");
                ui.label(RichText::new(crate::git::short(target)).monospace());
                ui.end_row();
                ui.label("Tag name:");
                text_field(ui, name, "v1.0.0", true);
                ui.end_row();
                ui.label("Message:");
                let r = ui.add(
                    egui::TextEdit::multiline(message)
                        .hint_text("Optional (creates an annotated tag)")
                        .desired_rows(3)
                        .desired_width(260.0),
                );
                let focused = r.has_focus();
                ui.ctx()
                    .data_mut(|d| d.insert_temp(egui::Id::new(MULTILINE_FOCUS), focused));
                ui.end_row();
                ui.label("");
                ui.add_enabled(
                    !remotes.is_empty(),
                    egui::Checkbox::new(push, "Push tag to remote"),
                );
                ui.end_row();
            });
            let valid = !name.trim().is_empty() && !name.contains(' ');
            let (ok, cancel) = buttons(ui, "Create Tag", valid, false);
            if ok {
                if let Some(tab) = tab {
                    let name = name.trim().to_owned();
                    let mut cmds = vec![if message.trim().is_empty() {
                        vec![s("tag"), name.clone(), target.clone()]
                    } else {
                        vec![
                            s("tag"),
                            s("-a"),
                            name.clone(),
                            s("-m"),
                            message.trim().to_owned(),
                            target.clone(),
                        ]
                    }];
                    if *push {
                        if let Some(remote) = tab.default_remote() {
                            cmds.push(vec![s("push"), remote, format!("refs/tags/{name}")]);
                        }
                    }
                    tab.git_seq(format!("Create tag {name}"), cmds);
                }
            }
            close = ok || cancel;
        }
        Dialog::Pull {
            remote,
            branch,
            rebase,
            autostash,
        } => {
            ui.heading("Pull");
            ui.add_space(6.0);
            grid(ui, "pull", |ui| {
                ui.label("Remote:");
                remote_combo(ui, "pull_remote", remote, &remotes);
                ui.end_row();
                ui.label("Branch:");
                text_field(ui, branch, "", false);
                ui.end_row();
                ui.label("");
                ui.checkbox(rebase, "Rebase instead of merge");
                ui.end_row();
                ui.label("");
                ui.checkbox(autostash, "Stash and reapply local changes");
                ui.end_row();
            });
            let (ok, cancel) = buttons(ui, "Pull", !remote.is_empty(), false);
            if ok {
                if let Some(tab) = tab {
                    let mut args = vec![
                        s("pull"),
                        if *rebase {
                            s("--rebase")
                        } else {
                            s("--no-rebase")
                        },
                    ];
                    if *autostash {
                        args.push(s("--autostash"));
                    }
                    args.push(remote.clone());
                    if !branch.trim().is_empty() {
                        args.push(branch.trim().to_owned());
                    }
                    tab.git("Pull", args);
                }
            }
            close = ok || cancel;
        }
        Dialog::Push {
            branch,
            remote,
            remote_branch,
            set_upstream,
            force,
            tags,
        } => {
            ui.heading("Push");
            ui.add_space(6.0);
            grid(ui, "push", |ui| {
                ui.label("Branch:");
                ui.label(RichText::new(branch.as_str()).monospace());
                ui.end_row();
                ui.label("To:");
                ui.horizontal(|ui| {
                    remote_combo(ui, "push_remote", remote, &remotes);
                });
                ui.end_row();
                ui.label("Remote branch:");
                text_field(ui, remote_branch, "", false);
                ui.end_row();
                ui.label("");
                ui.checkbox(set_upstream, "Set as upstream (track)");
                ui.end_row();
                ui.label("");
                ui.checkbox(tags, "Push all tags");
                ui.end_row();
                ui.label("");
                ui.checkbox(
                    force,
                    RichText::new("Force push (with lease)").color(if *force {
                        theme::pal(ui).removed
                    } else {
                        ui.visuals().text_color()
                    }),
                );
                ui.end_row();
            });
            let valid =
                !branch.is_empty() && !remote.is_empty() && !remote_branch.trim().is_empty();
            let (ok, cancel) = buttons(ui, "Push", valid, *force);
            if ok {
                if let Some(tab) = tab {
                    let mut args = vec![s("push")];
                    if *set_upstream {
                        args.push(s("-u"));
                    }
                    if *force {
                        args.push(s("--force-with-lease"));
                    }
                    if *tags {
                        args.push(s("--tags"));
                    }
                    args.push(remote.clone());
                    args.push(format!(
                        "refs/heads/{branch}:refs/heads/{}",
                        remote_branch.trim()
                    ));
                    tab.git("Push", args);
                }
            }
            close = ok || cancel;
        }
        Dialog::Stash {
            message,
            untracked,
            keep_index,
        } => {
            ui.heading("Stash Changes");
            ui.add_space(6.0);
            grid(ui, "stash", |ui| {
                ui.label("Message:");
                text_field(ui, message, "Optional", true);
                ui.end_row();
                ui.label("");
                ui.checkbox(untracked, "Include untracked files");
                ui.end_row();
                ui.label("");
                ui.checkbox(keep_index, "Keep staged changes");
                ui.end_row();
            });
            let (ok, cancel) = buttons(ui, "Stash", true, false);
            if ok {
                if let Some(tab) = tab {
                    let mut args = vec![s("stash"), s("push")];
                    if *untracked {
                        args.push(s("--include-untracked"));
                    }
                    if *keep_index {
                        args.push(s("--keep-index"));
                    }
                    if !message.trim().is_empty() {
                        args.push(s("-m"));
                        args.push(message.trim().to_owned());
                    }
                    tab.git("Stash", args);
                }
            }
            close = ok || cancel;
        }
        Dialog::Reset { target, mode } => {
            let branch = tab
                .as_ref()
                .and_then(|t| t.refs.head_branch.clone())
                .unwrap_or_else(|| "HEAD".into());
            ui.heading(format!("Reset '{branch}' to {}", crate::git::short(target)));
            ui.add_space(6.0);
            ui.radio_value(mode, ResetMode::Soft, "Soft — keep all changes staged");
            ui.radio_value(
                mode,
                ResetMode::Mixed,
                "Mixed — keep changes in the working tree, unstaged",
            );
            ui.radio_value(mode, ResetMode::Hard, "Hard — discard all local changes");
            if *mode == ResetMode::Hard {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("All uncommitted changes will be lost.")
                        .color(theme::pal(ui).removed),
                );
            }
            let (ok, cancel) = buttons(ui, "Reset", true, *mode == ResetMode::Hard);
            if ok {
                if let Some(tab) = tab {
                    let flag = match mode {
                        ResetMode::Soft => "--soft",
                        ResetMode::Mixed => "--mixed",
                        ResetMode::Hard => "--hard",
                    };
                    tab.git("Reset", vec![s("reset"), s(flag), target.clone()]);
                }
            }
            close = ok || cancel;
        }
        Dialog::Merge {
            source,
            no_ff,
            squash,
        } => {
            let branch = tab
                .as_ref()
                .and_then(|t| t.refs.head_branch.clone())
                .unwrap_or_else(|| "HEAD".into());
            ui.heading("Merge");
            ui.add_space(6.0);
            ui.label(format!("Merge '{source}' into '{branch}'"));
            ui.add_space(4.0);
            ui.checkbox(
                no_ff,
                "Create a merge commit even if fast-forward is possible (--no-ff)",
            );
            ui.checkbox(squash, "Squash commits (--squash)");
            let (ok, cancel) = buttons(ui, "Merge", true, false);
            if ok {
                if let Some(tab) = tab {
                    let mut args = vec![s("merge")];
                    if *squash {
                        args.push(s("--squash"));
                    } else {
                        args.push(s("--no-edit"));
                        if *no_ff {
                            args.push(s("--no-ff"));
                        }
                    }
                    args.push(source.clone());
                    if *squash {
                        // Remembered for the prefilled commit message.
                        tab.squash_source = Some(source.clone());
                    }
                    tab.git(format!("Merge {source}"), args);
                }
            }
            close = ok || cancel;
        }
        Dialog::AddRemote { name, url } => {
            ui.heading("Add Remote");
            ui.add_space(6.0);
            grid(ui, "ar", |ui| {
                ui.label("Name:");
                text_field(ui, name, "origin", true);
                ui.end_row();
                ui.label("URL:");
                text_field(ui, url, "git@github.com:owner/repo.git", false);
                ui.end_row();
            });
            let valid = !name.trim().is_empty() && !url.trim().is_empty();
            let (ok, cancel) = buttons(ui, "Add Remote", valid, false);
            if ok {
                if let Some(tab) = tab {
                    let n = name.trim().to_owned();
                    tab.git_seq(
                        format!("Add remote {n}"),
                        vec![
                            vec![s("remote"), s("add"), n.clone(), url.trim().to_owned()],
                            vec![s("fetch"), n],
                        ],
                    );
                }
            }
            close = ok || cancel;
        }
        Dialog::EditRemote {
            old_name,
            old_url,
            name,
            url,
        } => {
            ui.heading("Edit Remote");
            ui.add_space(6.0);
            grid(ui, "er", |ui| {
                ui.label("Name:");
                text_field(ui, name, "origin", true);
                ui.end_row();
                ui.label("URL:");
                text_field(ui, url, "git@github.com:owner/repo.git", false);
                ui.end_row();
            });
            let (n, u) = (name.trim().to_owned(), url.trim().to_owned());
            let changed = n != *old_name || u != *old_url;
            let valid = !n.is_empty() && !n.contains(' ') && !u.is_empty() && changed;
            let (ok, cancel) = buttons(ui, "Save", valid, false);
            if ok {
                if let Some(tab) = tab {
                    let mut cmds = Vec::new();
                    if n != *old_name {
                        cmds.push(vec![s("remote"), s("rename"), old_name.clone(), n.clone()]);
                    }
                    if u != *old_url {
                        cmds.push(vec![s("remote"), s("set-url"), n.clone(), u]);
                    }
                    tab.git_seq(format!("Edit remote {old_name}"), cmds);
                }
            }
            close = ok || cancel;
        }
        Dialog::Discard { files } => {
            ui.heading("Discard Changes");
            ui.add_space(6.0);
            if files.len() == 1 {
                ui.label(format!("Discard all changes to '{}'?", files[0].path));
            } else {
                ui.label(format!("Discard changes in {} files?", files.len()));
            }
            ui.label(RichText::new("This cannot be undone.").color(theme::pal(ui).removed));
            let (ok, cancel) = buttons(ui, "Discard", true, true);
            if ok {
                if let Some(tab) = tab {
                    tab.discard(files.clone());
                }
            }
            close = ok || cancel;
        }
        Dialog::DiscardPatch(selection) => {
            ui.heading("Discard Changes");
            ui.add_space(6.0);
            ui.label("Discard the selected changes from the working tree?");
            ui.label(RichText::new("This cannot be undone.").color(theme::pal(ui).removed));
            let (ok, cancel) = buttons(ui, "Discard", true, true);
            if ok {
                if let Some(tab) = tab {
                    tab.apply_partial(selection.clone(), PatchAction::Discard);
                }
            }
            close = ok || cancel;
        }
        Dialog::Confirm {
            title,
            text,
            button,
            destructive,
            label,
            commands,
        } => {
            ui.heading(title.as_str());
            ui.add_space(6.0);
            ui.label(text.as_str());
            let (ok, cancel) = buttons(ui, button, true, *destructive);
            if ok {
                if let Some(tab) = tab {
                    tab.git_seq(label.clone(), std::mem::take(commands));
                }
            }
            close = ok || cancel;
        }
        Dialog::Clone { url, parent, name } => {
            ui.heading("Clone Repository");
            ui.add_space(6.0);
            grid(ui, "clone", |ui| {
                ui.label("SSH or HTTPS address:");
                let before = url.clone();
                text_field(ui, url, "git@host:owner/repo.git or https://…", true);
                if *url != before {
                    let derived = url.trim().trim_end_matches('/').trim_end_matches(".git");
                    *name = derived
                        .rsplit(['/', ':'])
                        .next()
                        .unwrap_or_default()
                        .to_owned();
                }
                ui.end_row();
                ui.label("Parent folder:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(parent).desired_width(200.0));
                    if ui.button("Browse…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            *parent = dir.display().to_string();
                        }
                    }
                });
                ui.end_row();
                ui.label("Name:");
                text_field(ui, name, "", false);
                ui.end_row();
            });
            let valid =
                !url.trim().is_empty() && !parent.trim().is_empty() && !name.trim().is_empty();
            let (ok, cancel) = buttons(ui, "Clone", valid, false);
            if ok {
                app = Some(AppAction::Clone {
                    url: url.trim().to_owned(),
                    dest: PathBuf::from(parent.trim()).join(name.trim()),
                });
            }
            close = ok || cancel;
        }
        Dialog::RepoSettings {
            repo,
            original,
            edit,
            inherited,
            access,
        } => {
            let p = theme::pal(ui);
            ui.heading(format!("Repository Settings — {repo}"));
            ui.add_space(8.0);
            ui.label(RichText::new("Commit identity").strong());
            ui.add_space(2.0);
            grid(ui, "repo_identity", |ui| {
                ui.label("Name:");
                let hint = inherited
                    .user_name
                    .clone()
                    .unwrap_or_else(|| "Not set".into());
                text_field(ui, &mut edit.user_name, &hint, true);
                ui.end_row();
                ui.label("Email:");
                let hint = inherited
                    .user_email
                    .clone()
                    .unwrap_or_else(|| "Not set".into());
                text_field(ui, &mut edit.user_email, &hint, false);
                ui.end_row();
            });
            let email_ok = edit.user_email.trim().is_empty() || edit.user_email.contains('@');
            let effective_name = Some(edit.user_name.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or(inherited.user_name.clone());
            let effective_email = Some(edit.user_email.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or(inherited.user_email.clone());
            match (&effective_name, &effective_email) {
                _ if !email_ok => {
                    ui.label(
                        RichText::new("That doesn't look like an email address.")
                            .small()
                            .color(p.removed),
                    );
                }
                (Some(n), Some(e)) => {
                    ui.label(RichText::new(format!("Commits will be authored as {n} <{e}>. Leave empty to use your global identity.")).small().color(p.muted));
                }
                _ => {
                    ui.label(RichText::new("No identity configured: git will refuse to commit until a name and email are set.").small().color(p.modified));
                }
            }

            ui.add_space(10.0);
            grid(ui, "repo_behaviour", |ui| {
                ui.label(RichText::new("Pull").strong());
                egui::ComboBox::from_id_salt("repo_pull")
                    .width(260.0)
                    .selected_text(if edit.pull == PullMode::Inherit {
                        format!("Use global ({})", inherited.pull)
                    } else {
                        edit.pull.label().to_owned()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut edit.pull,
                            PullMode::Inherit,
                            format!("Use global ({})", inherited.pull),
                        );
                        for m in [PullMode::Merge, PullMode::Rebase, PullMode::FastForwardOnly] {
                            ui.selectable_value(&mut edit.pull, m, m.label());
                        }
                    });
                ui.end_row();
                ui.label(RichText::new("Fetch").strong());
                let global = if inherited.prune { "prune" } else { "keep" };
                let text = |v: Option<bool>| match v {
                    None => format!("Use global ({global})"),
                    Some(true) => "Prune deleted remote branches".to_owned(),
                    Some(false) => "Keep deleted remote branches".to_owned(),
                };
                egui::ComboBox::from_id_salt("repo_prune")
                    .width(260.0)
                    .selected_text(text(edit.prune))
                    .show_ui(ui, |ui| {
                        for v in [None, Some(true), Some(false)] {
                            ui.selectable_value(&mut edit.prune, v, text(v));
                        }
                    });
                ui.end_row();
            });
            ui.add_space(10.0);
            ui.label(RichText::new("Remote access").strong());
            ui.add_space(2.0);
            grid(ui, "repo_access", |ui| {
                // Account for HTTPS github.com remotes.
                ui.label("GitHub account:");
                let automatic = match &access.auto_account {
                    Some(login) => format!("Automatic (@{login})"),
                    None => "Automatic".to_owned(),
                };
                let selected = match &edit.github_account {
                    Some(login) => format!("@{login}"),
                    None => automatic.clone(),
                };
                let mut choices = access.accounts.clone();
                // Keep an account that is configured but not signed in here.
                if let Some(current) = &original.github_account {
                    if !choices.contains(current) {
                        choices.push(current.clone());
                    }
                }
                ui.add_enabled_ui(!choices.is_empty(), |ui| {
                    egui::ComboBox::from_id_salt("repo_github_account")
                        .width(260.0)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut edit.github_account, None, automatic);
                            for login in &choices {
                                ui.selectable_value(
                                    &mut edit.github_account,
                                    Some(login.clone()),
                                    format!("@{login}"),
                                );
                            }
                        });
                })
                .response
                .on_disabled_hover_text("Sign in to a GitHub account first");
                ui.end_row();

                // Key for SSH remotes.
                ui.label("SSH key:");
                let default = "Default (ssh agent / config)";
                let selected = match &edit.ssh_key {
                    SshKey::Default => default.to_owned(),
                    SshKey::Key(path) => short_path(path),
                    SshKey::Custom(_) => "Custom ssh command".to_owned(),
                };
                egui::ComboBox::from_id_salt("repo_ssh_key")
                    .width(260.0)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut edit.ssh_key, SshKey::Default, default);
                        for key in &access.ssh_keys {
                            ui.selectable_value(&mut edit.ssh_key, SshKey::Key(key.clone()), short_path(key));
                        }
                        // A key or command already configured that is not in ~/.ssh.
                        if !matches!(&original.ssh_key, SshKey::Key(k) if access.ssh_keys.contains(k)) && original.ssh_key != SshKey::Default {
                            let label = match &original.ssh_key {
                                SshKey::Key(path) => short_path(path),
                                _ => "Custom ssh command".to_owned(),
                            };
                            ui.selectable_value(&mut edit.ssh_key, original.ssh_key.clone(), label);
                        }
                    });
                ui.end_row();
            });
            let note = match (access.has_github_https, access.has_ssh) {
                (true, false) => {
                    "This repository's GitHub remote uses HTTPS, so the account applies; the SSH key is not used."
                }
                (false, true) => {
                    "This repository's remote uses SSH, so the key applies; the GitHub account is not used."
                }
                _ => "The account applies to HTTPS github.com remotes, the key to SSH remotes.",
            };
            ui.add(egui::Label::new(RichText::new(note).small().color(p.muted)).wrap());
            if let SshKey::Custom(command) = &edit.ssh_key {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("core.sshCommand = {command}"))
                            .small()
                            .monospace()
                            .color(p.muted),
                    )
                    .wrap(),
                );
            }

            ui.add_space(6.0);
            ui.label(
                RichText::new("Saved in this repository's .git/config (git config --local).")
                    .small()
                    .color(p.muted),
            );

            let changed = *edit != *original;
            let (ok, cancel) = buttons(ui, "Save", changed && email_ok, false);
            if ok {
                if let Some(tab) = tab {
                    let (old, new) = (original.clone(), edit.clone());
                    tab.run_task(
                        "Repository settings",
                        crate::repo::OpKind::Generic,
                        Box::new(move |repo| crate::git::save_repo_config(repo, &old, &new)),
                    );
                }
            }
            close = ok || cancel;
        }
        Dialog::Workspaces(edits) => {
            let p = theme::pal(ui);
            ui.heading("Workspaces");
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Each workspace has its own repository list and open tabs.")
                        .small()
                        .color(p.muted),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            let count = edits.len();
            let mut remove = None;
            let mut moved = None;
            for (i, (_, name)) in edits.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(name)
                            .hint_text("Workspace name")
                            .desired_width(250.0),
                    );
                    use egui_phosphor::regular as icon;
                    if ui
                        .add_enabled(i > 0, egui::Button::new(icon::ARROW_UP))
                        .on_hover_text("Move up")
                        .clicked()
                    {
                        moved = Some((i, i - 1));
                    }
                    if ui
                        .add_enabled(i + 1 < count, egui::Button::new(icon::ARROW_DOWN))
                        .on_hover_text("Move down")
                        .clicked()
                    {
                        moved = Some((i, i + 1));
                    }
                    if ui
                        .add_enabled(count > 1, egui::Button::new(icon::TRASH))
                        .on_hover_text(
                            "Remove this workspace (repositories on disk are not touched)",
                        )
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            }
            if let Some((from, to)) = moved {
                edits.swap(from, to);
            }
            if let Some(i) = remove {
                edits.remove(i);
            }
            ui.add_space(4.0);
            if ui
                .button(format!("{} Add Workspace", egui_phosphor::regular::PLUS))
                .clicked()
            {
                edits.push((None, String::new()));
            }
            let problem = crate::workspace::validate(edits);
            if let Some(problem) = problem {
                ui.add_space(4.0);
                ui.label(RichText::new(problem).small().color(p.removed));
            }
            let (ok, cancel) = buttons(ui, "Save", problem.is_none(), false);
            if ok {
                app = Some(AppAction::SaveWorkspaces(edits.clone()));
            }
            close = ok || cancel;
        }
        Dialog::Settings(settings) => {
            ui.heading("Settings");
            ui.add_space(6.0);
            grid(ui, "settings", |ui| {
                ui.label("Theme:");
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut settings.theme,
                        crate::app::ThemeChoice::System,
                        "System",
                    );
                    ui.selectable_value(&mut settings.theme, crate::app::ThemeChoice::Dark, "Dark");
                    ui.selectable_value(
                        &mut settings.theme,
                        crate::app::ThemeChoice::Light,
                        "Light",
                    );
                });
                ui.end_row();
                ui.label("Git executable:");
                ui.add(
                    egui::TextEdit::singleline(&mut settings.git_path)
                        .hint_text("git (from PATH)")
                        .desired_width(260.0),
                );
                ui.end_row();
                ui.label("SSH executable:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut settings.ssh_path)
                            .hint_text("ssh (from PATH)")
                            .desired_width(226.0),
                    )
                    .on_hover_text(
                        "The ssh program git uses for SSH remotes. Set this if you need a specific \
                         build, e.g. Homebrew's ssh for hardware security keys.",
                    );
                    // Offer the ssh programs found in the usual places.
                    ui.menu_button(egui_phosphor::regular::CARET_DOWN, |ui| {
                        if ui.button("ssh (from PATH)").clicked() {
                            settings.ssh_path.clear();
                            ui.close();
                        }
                        for candidate in [
                            "/opt/homebrew/bin/ssh",
                            "/usr/local/bin/ssh",
                            "/usr/bin/ssh",
                        ] {
                            if std::path::Path::new(candidate).is_file()
                                && ui.button(candidate).clicked()
                            {
                                settings.ssh_path = candidate.to_owned();
                                ui.close();
                            }
                        }
                    });
                });
                ui.end_row();
                ui.label("Max commits loaded:");
                ui.add(
                    egui::DragValue::new(&mut settings.commit_limit)
                        .range(100..=500_000)
                        .speed(100),
                );
                ui.end_row();
                ui.label("Diff font size:");
                ui.add(
                    egui::DragValue::new(&mut settings.diff_font_size)
                        .range(9.0..=24.0)
                        .speed(0.25),
                );
                ui.end_row();
                ui.label("UI zoom:");
                ui.add(egui::Slider::new(&mut settings.zoom, 0.75..=1.75).step_by(0.05));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "{} — changes to the git executable apply after restart.",
                    crate::git::cmd::git_version().unwrap_or_else(|| "git not found".into())
                ))
                .small()
                .color(theme::pal(ui).muted),
            );
            let ssh = settings.ssh_path.trim();
            let ssh_ok = ssh.is_empty() || std::path::Path::new(ssh).is_file();
            if !ssh_ok {
                ui.label(
                    RichText::new(format!("SSH executable not found: {ssh}"))
                        .small()
                        .color(theme::pal(ui).removed),
                );
            }
            let (ok, cancel) = buttons(ui, "Save", ssh_ok, false);
            if ok {
                app = Some(AppAction::SaveSettings(settings.clone()));
            }
            close = ok || cancel;
        }
    }
    match (app, close) {
        (Some(a), _) => Outcome::App(a),
        (None, true) => Outcome::Close,
        (None, false) => Outcome::Open,
    }
}
