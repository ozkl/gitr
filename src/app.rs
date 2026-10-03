//! Top-level application: repository manager, tabs, toolbar, dialogs, notifications.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

use egui::{Align2, Color32, FontId, Key, KeyboardShortcut, Modifiers, Pos2, Rect, RichText, Sense, Vec2};
use egui_phosphor::regular as icon;
use serde::{Deserialize, Serialize};

use crate::git::{self, cmd};
use crate::repo::{RepoTab, View};
use crate::ui::dialogs::{self, AppAction, Dialog, Outcome};
use crate::ui::{changes, history, sidebar, theme, Ctx};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub git_path: String,
    pub commit_limit: usize,
    pub diff_font_size: f32,
    /// Show local changes grouped by folder.
    pub changes_tree_view: bool,
    pub zoom: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: ThemeChoice::System, git_path: String::new(), commit_limit: 20_000, diff_font_size: 12.5, zoom: 1.0, changes_tree_view: false }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Persisted {
    repos: Vec<PathBuf>,
    open_tabs: Vec<PathBuf>,
    active_tab: Option<usize>,
    settings: Settings,
    hide_repo_list: bool,
}

const STORAGE_KEY: &str = "gitr_state";

struct Toast {
    title: String,
    text: String,
    error: bool,
    created: Instant,
}

struct PendingClone {
    url: String,
    rx: Receiver<Result<PathBuf, String>>,
}

pub struct GitrApp {
    repos: Vec<PathBuf>,
    tabs: Vec<RepoTab>,
    active: Option<usize>,
    settings: Settings,
    show_repo_list: bool,
    dialog: Option<Dialog>,
    clones: Vec<PendingClone>,
    toasts: Vec<Toast>,
    show_activity: bool,
    was_focused: bool,
    repo_filter: String,
    git_version: Option<String>,
    logo: egui::TextureHandle,
    #[cfg(feature = "screenshot")]
    devshot: crate::devshot::DevShot,
}

impl GitrApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply_style(&cc.egui_ctx);
        let persisted: Persisted = cc.storage.and_then(|s| eframe::get_value(s, STORAGE_KEY)).unwrap_or_default();
        cmd::set_git_binary(&persisted.settings.git_path);
        let mut app = Self {
            repos: persisted.repos.into_iter().filter(|p| p.exists()).collect(),
            tabs: Vec::new(),
            active: None,
            settings: persisted.settings,
            show_repo_list: !persisted.hide_repo_list,
            dialog: None,
            clones: Vec::new(),
            toasts: Vec::new(),
            show_activity: false,
            was_focused: true,
            repo_filter: String::new(),
            git_version: cmd::git_version(),
            logo: load_logo(&cc.egui_ctx),
            #[cfg(feature = "screenshot")]
            devshot: crate::devshot::DevShot::from_env(),
        };
        app.apply_settings(&cc.egui_ctx);
        // Dev aid: draw extra windows inside the main one so screenshots include them.
        #[cfg(feature = "screenshot")]
        if std::env::var_os("GITR_EMBED_VIEWPORTS").is_some() {
            cc.egui_ctx.set_embed_viewports(true);
        }
        if app.git_version.is_none() {
            app.toast("Git not found", "Install git and make sure it is on your PATH, or set its location in Settings.", true);
        }
        for path in persisted.open_tabs.iter().filter(|p| p.exists()) {
            app.tabs.push(RepoTab::open(path.clone(), cc.egui_ctx.clone(), app.settings.commit_limit));
        }
        app.active = persisted.active_tab.filter(|&i| i < app.tabs.len()).or(if app.tabs.is_empty() { None } else { Some(0) });
        // Command-line argument: open a repository.
        if let Some(arg) = std::env::args().nth(1) {
            app.open_repo(&cc.egui_ctx, Path::new(&arg));
        }
        app
    }

    fn apply_settings(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
        });
        ctx.set_zoom_factor(self.settings.zoom);
    }

    fn toast(&mut self, title: impl Into<String>, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { title: title.into(), text: text.into(), error, created: Instant::now() });
    }

    fn open_repo(&mut self, ctx: &egui::Context, path: &Path) {
        if !path.is_dir() {
            self.toast("Folder not found", path.display().to_string(), true);
            return;
        }
        let root = match git::repo_root(path) {
            Ok(root) => root,
            Err(e) => {
                self.toast("Not a git repository", format!("{}\n{e}", path.display()), true);
                return;
            }
        };
        if !self.repos.contains(&root) {
            self.repos.push(root.clone());
        }
        if let Some(i) = self.tabs.iter().position(|t| t.path == root) {
            self.active = Some(i);
            return;
        }
        self.tabs.push(RepoTab::open(root, ctx.clone(), self.settings.commit_limit));
        self.active = Some(self.tabs.len() - 1);
    }

    fn close_tab(&mut self, i: usize) {
        self.tabs.remove(i);
        self.active = match self.active {
            _ if self.tabs.is_empty() => None,
            Some(a) if a > i => Some(a - 1),
            Some(a) if a == i => Some(i.min(self.tabs.len() - 1)),
            other => other,
        };
    }

    fn pick_and_open(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Open Repository").pick_folder() {
            self.open_repo(ctx, &dir);
        }
    }

    fn init_repo(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Create Repository In").pick_folder() {
            match cmd::run(&dir, &["init"]) {
                Ok(_) => self.open_repo(ctx, &dir),
                Err(e) => self.toast("git init failed", e, true),
            }
        }
    }

    fn clone_dialog(&self) -> Dialog {
        let parent = self
            .repos
            .last()
            .and_then(|p| p.parent())
            .map(|p| p.display().to_string())
            .or_else(|| std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok()))
            .unwrap_or_default();
        Dialog::Clone { url: String::new(), parent, name: String::new() }
    }

    fn start_clone(&mut self, ctx: &egui::Context, url: String, dest: PathBuf) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let url2 = url.clone();
        std::thread::spawn(move || {
            let parent = dest.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
            let _ = std::fs::create_dir_all(&parent);
            let dest_s = dest.display().to_string();
            let result = cmd::run_full(&parent, &["clone", "--recurse-submodules", &url2, &dest_s]).map(|_| dest);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.clones.push(PendingClone { url, rx });
    }

    fn handle_app_action(&mut self, ctx: &egui::Context, action: AppAction) {
        match action {
            AppAction::Clone { url, dest } => self.start_clone(ctx, url, dest),
            AppAction::SaveSettings(s) => {
                let limit_changed = s.commit_limit != self.settings.commit_limit;
                self.settings = s;
                self.apply_settings(ctx);
                for t in &mut self.tabs {
                    t.commit_limit = self.settings.commit_limit;
                    if limit_changed {
                        t.reload_log();
                    }
                }
            }
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            // Returning to the app: pick up changes made elsewhere.
            if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
                tab.refresh();
                tab.reload_change_diff();
            }
        }
        self.was_focused = focused;
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            tab.tick(focused && Some(i) == self.active);
            for n in tab.notices.drain(..) {
                self.toasts.push(Toast { title: n.title, text: n.text, error: n.error, created: Instant::now() });
            }
        }
        let mut done = Vec::new();
        self.clones.retain(|c| match c.rx.try_recv() {
            Ok(r) => {
                done.push((c.url.clone(), r));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for (url, r) in done {
            match r {
                Ok(path) => self.open_repo(ctx, &path),
                Err(e) => self.toast(format!("Clone of {url} failed"), e, true),
            }
        }
        self.toasts.retain(|t| t.error || t.created.elapsed().as_secs() < 5);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) -> (bool, bool) {
        let sc = |m, k| KeyboardShortcut::new(m, k);
        let mut focus_search = false;
        let mut focus_commit = false;
        if self.dialog.is_some() {
            return (false, false);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::O))) {
            self.pick_and_open(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::W))) {
            if let Some(i) = self.active {
                self.close_tab(i);
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::CTRL, Key::Tab))) && !self.tabs.is_empty() {
            self.active = Some(self.active.map_or(0, |a| (a + 1) % self.tabs.len()));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab))) && !self.tabs.is_empty() {
            self.active = Some(self.active.map_or(0, |a| (a + self.tabs.len() - 1) % self.tabs.len()));
        }
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else {
            return (false, false);
        };
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::R)) || i.consume_key(Modifiers::NONE, Key::F5)) {
            tab.refresh();
            tab.reload_change_diff();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::F))) {
            if tab.view == View::LocalChanges {
                tab.focus_changes_filter = true;
            } else {
                focus_search = true;
            }
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Num1))) {
            tab.view = View::LocalChanges;
            focus_commit = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND, Key::Num2))) {
            tab.view = View::History;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::F))) {
            tab.fetch_all();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::L))) {
            self.dialog = Some(Dialog::pull(tab));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::P))) {
            self.dialog = Some(Dialog::push(tab));
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::B))) {
            if let Some(head) = tab.refs.head_id.clone() {
                let label = tab.refs.head_branch.clone().unwrap_or_else(|| git::short(&head).to_owned());
                self.dialog = Some(Dialog::CreateBranch { start: head, start_label: label, name: String::new(), checkout: true });
            }
        }
        (focus_search, focus_commit)
    }

    // ------------------------------------------------------------------
    // Panels

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if tool_button(ui, icon::SIDEBAR_SIMPLE, "Repos", true, self.show_repo_list).on_hover_text("Toggle repository list").clicked() {
                self.show_repo_list = !self.show_repo_list;
            }
            ui.add_space(8.0);
            let tab = self.active.and_then(|i| self.tabs.get_mut(i));
            let has = tab.is_some();
            let (has_remote, has_stash, has_head) = tab
                .as_ref()
                .map(|t| (!t.refs.remotes.is_empty(), !t.refs.stashes.is_empty(), t.refs.head_branch.is_some()))
                .unwrap_or_default();
            let mut tab = tab;
            if tool_button(ui, icon::ARROWS_CLOCKWISE, "Fetch", has && has_remote, false).on_hover_text("Fetch all remotes (⇧⌘F)").clicked() {
                if let Some(t) = tab.as_deref_mut() {
                    t.fetch_all();
                }
            }
            if tool_button(ui, icon::ARROW_DOWN, "Pull", has && has_remote, false).on_hover_text("Pull (⇧⌘L)").clicked() {
                if let Some(t) = tab.as_deref() {
                    self.dialog = Some(Dialog::pull(t));
                }
            }
            if tool_button(ui, icon::ARROW_UP, "Push", has && has_remote && has_head, false).on_hover_text("Push (⇧⌘P)").clicked() {
                if let Some(t) = tab.as_deref() {
                    self.dialog = Some(Dialog::push(t));
                }
            }
            ui.add_space(8.0);
            let dirty = tab.as_ref().is_some_and(|t| !t.status.is_clean());
            if tool_button(ui, icon::ARCHIVE_BOX, "Stash", has && dirty, false).clicked() {
                self.dialog = Some(Dialog::Stash { message: String::new(), untracked: true, keep_index: false });
            }
            if tool_button(ui, icon::TRAY_ARROW_UP, "Pop", has && has_stash, false).on_hover_text("Pop latest stash").clicked() {
                if let Some(t) = tab.as_deref_mut() {
                    t.git("Pop stash", vec!["stash".into(), "pop".into()]);
                }
            }

            // Center: repository + branch.
            let right_w = 4.0 * 58.0;
            let center_w = (ui.available_width() - right_w - 16.0).max(0.0);
            ui.allocate_ui_with_layout(Vec2::new(center_w, 44.0), egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                if let Some(t) = tab.as_deref_mut() {
                    branch_switcher(ui, t, &mut self.dialog);
                } else {
                    ui.label(RichText::new("Gitr").size(15.0).color(p.muted));
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tool_button(ui, icon::GEAR, "Settings", true, false).clicked() {
                    self.dialog = Some(Dialog::Settings(self.settings.clone()));
                }
                if tool_button(ui, icon::TERMINAL_WINDOW, "Terminal", has, false).clicked() {
                    if let Some(t) = tab.as_deref() {
                        crate::platform::open_terminal(&t.path);
                    }
                }
                if tool_button(ui, icon::FOLDER_OPEN, "Finder", has, false).on_hover_text("Show in file manager").clicked() {
                    if let Some(t) = tab.as_deref() {
                        t.open_in_file_manager(None);
                    }
                }
                if tool_button(ui, icon::GIT_BRANCH, "Branch", has && tab.as_ref().is_some_and(|t| t.refs.head_id.is_some()), false)
                    .on_hover_text("Create branch (⇧⌘B)")
                    .clicked()
                {
                    if let Some(t) = tab.as_deref() {
                        if let Some(head) = t.refs.head_id.clone() {
                            let label = t.refs.head_branch.clone().unwrap_or_else(|| git::short(&head).to_owned());
                            self.dialog = Some(Dialog::CreateBranch { start: head, start_label: label, name: String::new(), checkout: true });
                        }
                    }
                }
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let mut activate = None;
        let mut close = None;
        let mut action: Option<u8> = None;
        let mut repo_settings = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let home_sel = self.active.is_none();
            if ui.add(egui::Button::selectable(home_sel, format!("{}", icon::HOUSE))).on_hover_text("Home").clicked() {
                self.active = None;
            }
            let n = self.tabs.len().max(1) as f32;
            let tab_w = ((ui.available_width() - 40.0) / n).clamp(80.0, 220.0);
            for (i, tab) in self.tabs.iter().enumerate() {
                let selected = self.active == Some(i);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(tab_w, 26.0), Sense::click());
                let fill = if selected { p.selection.gamma_multiply(0.9) } else if resp.hovered() { p.hover } else { p.sidebar };
                ui.painter().rect(rect, 13.0, fill, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
                let color = if selected { Color32::WHITE } else { ui.visuals().text_color() };
                let label = if tab.is_busy() { format!("{} {}", icon::SPINNER, tab.name) } else { tab.name.clone() };
                let galley = ui.painter().layout_no_wrap(label, FontId::proportional(13.0), color);
                let clip = rect.shrink2(Vec2::new(22.0, 0.0));
                let x = (rect.center().x - galley.size().x / 2.0).max(clip.left());
                ui.painter().with_clip_rect(clip).galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, color);
                if tab.is_busy() {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                }
                // Close button.
                let close_rect = Rect::from_center_size(Pos2::new(rect.right() - 13.0, rect.center().y), Vec2::splat(16.0));
                let close_resp = ui.interact(close_rect, ui.id().with(("close_tab", i)), Sense::click());
                // Settings button, mirroring the close button on the left edge.
                let gear_rect = Rect::from_center_size(Pos2::new(rect.left() + 13.0, rect.center().y), Vec2::splat(16.0));
                let gear_resp = ui
                    .interact(gear_rect, ui.id().with(("tab_settings", i)), Sense::click())
                    .on_hover_text("Repository Settings");
                let show_buttons = resp.hovered() || close_resp.hovered() || gear_resp.hovered() || selected;
                if show_buttons {
                    let c = if close_resp.hovered() { color } else { color.gamma_multiply(0.6) };
                    ui.painter().text(close_rect.center(), Align2::CENTER_CENTER, icon::X, FontId::proportional(12.0), c);
                    let c = if gear_resp.hovered() { color } else { color.gamma_multiply(0.6) };
                    ui.painter().text(gear_rect.center(), Align2::CENTER_CENTER, icon::GEAR, FontId::proportional(13.0), c);
                }
                if gear_resp.clicked() {
                    repo_settings = Some(i);
                } else if close_resp.clicked() || resp.middle_clicked() {
                    close = Some(i);
                } else if resp.clicked() {
                    activate = Some(i);
                }
                let resp = resp.on_hover_text(tab.path.display().to_string());
                resp.context_menu(|ui| {
                    if ui.button("Close Tab").clicked() {
                        close = Some(i);
                        ui.close();
                    }
                    if ui.button("Show in file manager").clicked() {
                        tab.open_in_file_manager(None);
                        ui.close();
                    }
                    if ui.button("Repository Settings…").clicked() {
                        repo_settings = Some(i);
                        ui.close();
                    }
                });
            }
            ui.menu_button(icon::PLUS, |ui| {
                if ui.button(format!("{} Open Repository…", icon::FOLDER_OPEN)).clicked() {
                    action = Some(0);
                    ui.close();
                }
                if ui.button(format!("{} Clone Repository…", icon::DOWNLOAD_SIMPLE)).clicked() {
                    action = Some(1);
                    ui.close();
                }
                if ui.button(format!("{} New Repository…", icon::FOLDER_PLUS)).clicked() {
                    action = Some(2);
                    ui.close();
                }
            });
        });
        if let Some(i) = activate {
            self.active = Some(i);
        }
        if let Some(i) = repo_settings {
            self.active = Some(i);
            self.dialog = Some(Dialog::repo_settings(&self.tabs[i]));
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        let ctx = ui.ctx().clone();
        match action {
            Some(0) => self.pick_and_open(&ctx),
            Some(1) => self.dialog = Some(self.clone_dialog()),
            Some(2) => self.init_repo(&ctx),
            _ => {}
        }
    }

    fn repo_list(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Repositories").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button(icon::PLUS).on_hover_text("Open repository").clicked() {
                    let ctx = ui.ctx().clone();
                    self.pick_and_open(&ctx);
                }
            });
        });
        ui.add(egui::TextEdit::singleline(&mut self.repo_filter).hint_text(format!("{} Filter", icon::MAGNIFYING_GLASS)).desired_width(f32::INFINITY));
        ui.add_space(4.0);
        let filter = self.repo_filter.to_lowercase();
        let mut open = None;
        let mut remove = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for (i, path) in self.repos.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                    continue;
                }
                let active = self.active.and_then(|a| self.tabs.get(a)).is_some_and(|t| t.path == *path);
                let is_open = self.tabs.iter().any(|t| t.path == *path);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                if active {
                    ui.painter().rect_filled(rect, 5.0, p.hover.gamma_multiply(1.4));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 5.0, p.hover);
                }
                if is_open {
                    ui.painter().circle_filled(Pos2::new(rect.left() + 8.0, rect.center().y), 3.0, p.muted);
                }
                ui.painter().text(
                    Pos2::new(rect.left() + 18.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    &name,
                    FontId::proportional(14.0),
                    ui.visuals().text_color(),
                );
                let resp = resp.on_hover_text(path.display().to_string());
                if resp.clicked() {
                    open = Some(path.clone());
                }
                resp.context_menu(|ui| {
                    if ui.button("Open").clicked() {
                        open = Some(path.clone());
                        ui.close();
                    }
                    if ui.button("Show in file manager").clicked() {
                        crate::platform::reveal(path);
                        ui.close();
                    }
                    if ui.button("Open in terminal").clicked() {
                        crate::platform::open_terminal(path);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Remove from list").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
        });
        let ctx = ui.ctx().clone();
        if let Some(p) = open {
            self.open_repo(&ctx, &p);
        }
        if let Some(i) = remove {
            self.repos.remove(i);
        }
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.2).max(20.0));
            ui.add(egui::Image::new((self.logo.id(), Vec2::splat(112.0))));
            ui.label(RichText::new("Gitr").size(30.0).strong());
            ui.label(RichText::new(self.git_version.clone().unwrap_or_else(|| "git not found".into())).color(p.muted));
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                let w = 3.0 * 170.0;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                if big_button(ui, icon::FOLDER_OPEN, "Open Repository").clicked() {
                    self.pick_and_open(&ctx);
                }
                if big_button(ui, icon::DOWNLOAD_SIMPLE, "Clone Repository").clicked() {
                    self.dialog = Some(self.clone_dialog());
                }
                if big_button(ui, icon::FOLDER_PLUS, "New Repository").clicked() {
                    self.init_repo(&ctx);
                }
            });
            ui.add_space(24.0);
            if !self.repos.is_empty() {
                ui.label(RichText::new("Recent").strong().color(p.muted));
                ui.add_space(4.0);
                let mut open = None;
                for path in self.repos.iter().rev().take(10) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let r = ui.add(egui::Button::new(RichText::new(format!("{name}   ")).size(14.0)).right_text(RichText::new(path.display().to_string()).color(p.muted)).frame(false));
                    if r.clicked() {
                        open = Some(path.clone());
                    }
                }
                if let Some(path) = open {
                    self.open_repo(&ctx, &path);
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Tip: drop a folder onto this window to open it.").small().color(p.muted));
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::pal(ui);
        ui.horizontal(|ui| {
            if let Some(tab) = self.active.and_then(|i| self.tabs.get(i)) {
                if let Some(op) = tab.running.first() {
                    ui.spinner();
                    ui.label(format!("{op}…"));
                } else if let Some(last) = tab.activity.last() {
                    let color = if last.ok { p.muted } else { p.removed };
                    ui.label(RichText::new(format!("{} {} — {}", if last.ok { icon::CHECK } else { icon::WARNING }, last.label, last.time.format("%H:%M:%S"))).color(color));
                }
                if let Some(branch) = tab.refs.head_upstream() {
                    if let Some(up) = &branch.upstream {
                        ui.separator();
                        ui.label(RichText::new(format!("{} → {up}", branch.name)).color(p.muted));
                    }
                }
            }
            for c in &self.clones {
                ui.separator();
                ui.spinner();
                ui.label(format!("Cloning {}…", c.url));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut self.show_activity, format!("{} Activity", icon::LIST));
            });
        });
    }

    fn activity_window(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else { return };
        let mut open = self.show_activity;
        egui::Window::new(format!("Activity — {}", tab.name))
            .open(&mut open)
            .default_size([560.0, 360.0])
            .show(ctx, |ui| {
                let p = theme::pal(ui);
                if ui.small_button("Clear").clicked() {
                    tab.activity.clear();
                }
                egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show(ui, |ui| {
                    for a in &tab.activity {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(if a.ok { icon::CHECK } else { icon::X }).color(if a.ok { p.added } else { p.removed }));
                            ui.label(RichText::new(a.time.format("%H:%M:%S").to_string()).color(p.muted));
                            ui.label(RichText::new(&a.label).strong());
                        });
                        if !a.output.is_empty() {
                            ui.label(RichText::new(&a.output).monospace().small());
                        }
                        ui.separator();
                    }
                });
            });
        self.show_activity = open;
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let mut remove = None;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-12.0, -36.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let p = theme::pal(ui);
                ui.set_max_width(420.0);
                for (i, t) in self.toasts.iter().enumerate() {
                    egui::Frame::popup(ui.style())
                        .stroke(egui::Stroke::new(1.0, if t.error { p.removed } else { p.border }))
                        .show(ui, |ui| {
                            ui.set_width(400.0);
                            ui.horizontal(|ui| {
                                let (ic, color) = if t.error { (icon::WARNING, p.removed) } else { (icon::INFO, p.renamed) };
                                ui.label(RichText::new(ic).color(color));
                                ui.label(RichText::new(&t.title).strong());
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button(icon::X).clicked() {
                                        remove = Some(i);
                                    }
                                    if ui.small_button(icon::COPY).on_hover_text("Copy message").clicked() {
                                        ui.ctx().copy_text(t.text.clone());
                                    }
                                });
                            });
                            egui::ScrollArea::vertical().id_salt(("toast", i)).max_height(160.0).show(ui, |ui| {
                                ui.label(RichText::new(&t.text).monospace().small());
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = remove {
            self.toasts.remove(i);
        }
    }
}

#[cfg(feature = "screenshot")]
impl GitrApp {
    fn apply_dev_actions(&mut self) {
        if self.devshot.actions_applied {
            return;
        }
        let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) else { return };
        if !tab.log_loaded {
            return;
        }
        let needs_details = self.devshot.actions.iter().any(|a| a == "expand_all");
        if needs_details && !matches!(tab.details, Some(crate::repo::Loaded::Ready(_))) {
            return;
        }
        self.devshot.actions_applied = true;
        for action in self.devshot.actions.clone() {
            let (key, value) = action.split_once('=').unwrap_or((&action, ""));
            match key {
                "changes" => tab.view = View::LocalChanges,
                "first_change" => {
                    let first = tab
                        .status
                        .unstaged
                        .first()
                        .map(|f| (false, f.path.clone()))
                        .or_else(|| tab.status.staged.first().map(|f| (true, f.path.clone())));
                    if let Some((staged, path)) = first {
                        tab.select_change(staged, path);
                    }
                }
                "tab" => {
                    tab.detail_tab = match value {
                        "changes" => crate::repo::DetailTab::Changes,
                        "tree" => crate::repo::DetailTab::FileTree,
                        _ => crate::repo::DetailTab::Commit,
                    }
                }
                "select" => {
                    if let Some(c) = value.parse::<usize>().ok().and_then(|i| tab.commits.get(i)) {
                        let id = c.id.clone();
                        tab.select_commit(id);
                    }
                }
                "search" => tab.search = value.to_owned(),
                "expand" => tab.expanded_files.insert(value.to_owned()).then_some(()).unwrap_or(()),
                "tree_file" => tab.load_tree_file(value.to_owned()),
                "light" => self.settings.theme = ThemeChoice::Light,
                "home" => self.active = None,
                "merge_editor" => {
                    tab.select_change(false, value.to_owned());
                    let _ = tab.open_merge_editor(value);
                }
                "expand_all" => {
                    if let Some(crate::repo::Loaded::Ready(d)) = &tab.details {
                        let files = d.files.clone();
                        for f in &files {
                            tab.expanded_files.insert(f.path.clone());
                            tab.load_commit_diff(f);
                        }
                    }
                }
                "history" => tab.open_file_history("HEAD", value),
                "tree_view" => self.settings.changes_tree_view = true,
                "select_all" => tab.select_all_changes(false),
                "select_folder" => {
                    let paths: Vec<String> = tab.status.unstaged.iter().filter(|f| f.path.starts_with(value)).map(|f| f.path.clone()).collect();
                    tab.select_changes(false, paths, false);
                }
                "filter" => {
                        tab.changes_filter = value.to_owned();
                }
                "select_change" => tab.select_change(false, value.to_owned()),
                "dialog" => {
                    self.dialog = Some(match value {
                        "push" => Dialog::push(tab),
                        "pull" => Dialog::pull(tab),
                        "settings" => Dialog::Settings(self.settings.clone()),
                        "repo_settings" => Dialog::repo_settings(tab),
                        "edit_remote" => {
                            let r = tab.refs.remotes.first().cloned().expect("a remote");
                            Dialog::EditRemote { old_name: r.name.clone(), old_url: r.url.clone(), name: r.name, url: r.url }
                        }
                        "clone" => Dialog::Clone { url: String::new(), parent: String::new(), name: String::new() },
                        _ => Dialog::Stash { message: String::new(), untracked: true, keep_index: false },
                    })
                }
                _ => eprintln!("unknown dev action {action}"),
            }
        }
    }
}

fn load_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let icon = eframe::icon_data::from_png_bytes(crate::ICON_PNG).unwrap_or_default();
    let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    ctx.load_texture("gitr_logo", image, egui::TextureOptions::LINEAR)
}

fn tool_button(ui: &mut egui::Ui, glyph: &str, label: &str, enabled: bool, active: bool) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(56.0, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && (resp.hovered() || active) {
        ui.painter().rect_filled(rect, 6.0, p.hover);
    }
    let color = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color().gamma_multiply(0.6) };
    ui.painter().text(Pos2::new(rect.center().x, rect.top() + 14.0), Align2::CENTER_CENTER, glyph, FontId::proportional(19.0), color);
    ui.painter().text(Pos2::new(rect.center().x, rect.bottom() - 9.0), Align2::CENTER_CENTER, label, FontId::proportional(11.5), color);
    resp
}

fn big_button(ui: &mut egui::Ui, glyph: &str, label: &str) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(160.0, 90.0), Sense::click());
    let fill = if resp.hovered() { p.hover } else { p.sidebar };
    ui.painter().rect(rect, 10.0, fill, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    ui.painter().text(Pos2::new(rect.center().x, rect.top() + 34.0), Align2::CENTER_CENTER, glyph, FontId::proportional(28.0), p.selection);
    ui.painter().text(Pos2::new(rect.center().x, rect.bottom() - 20.0), Align2::CENTER_CENTER, label, FontId::proportional(13.5), ui.visuals().text_color());
    resp
}

fn branch_switcher(ui: &mut egui::Ui, tab: &mut RepoTab, dialog: &mut Option<Dialog>) {
    let p = theme::pal(ui);
    let branch = tab
        .refs
        .head_branch
        .clone()
        .unwrap_or_else(|| tab.refs.head_id.as_deref().map(|h| format!("detached at {}", git::short(h))).unwrap_or_else(|| "no commits".into()));
    let status = tab.refs.head_upstream().map(|b| {
        let mut s = String::new();
        if b.ahead > 0 {
            s.push_str(&format!("  {}{}", icon::ARROW_UP, b.ahead));
        }
        if b.behind > 0 {
            s.push_str(&format!("  {}{}", icon::ARROW_DOWN, b.behind));
        }
        s
    });
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(&tab.name).strong().size(14.0));
        let text = format!("{} {branch}{}  {}", icon::GIT_BRANCH, status.unwrap_or_default(), icon::CARET_DOWN);
        let r = ui.add(egui::Button::new(RichText::new(text).color(p.muted)).frame(false));
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(220.0);
            ui.label(RichText::new("Switch branch").small().color(p.muted));
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                let branches = tab.refs.branches.clone();
                for b in branches {
                    let label = if b.is_head { format!("{} {}", icon::CHECK, b.name) } else { format!("    {}", b.name) };
                    if ui.add_enabled(!b.is_head, egui::Button::new(label).frame(false)).clicked() {
                        tab.checkout(&b.name);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui.button(format!("{} New Branch…", icon::PLUS)).clicked() {
                if let Some(head) = tab.refs.head_id.clone() {
                    *dialog = Some(Dialog::CreateBranch { start: head, start_label: branch.clone(), name: String::new(), checkout: true });
                }
                ui.close();
            }
        });
    });
}

impl eframe::App for GitrApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_background(&ctx);
        #[cfg(feature = "screenshot")]
        {
            self.devshot.tick(&ctx);
            self.apply_dev_actions();
        }

        // Drag & drop folders to open them.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            self.open_repo(&ctx, &path);
        }

        let (focus_search, focus_commit) = self.shortcuts(&ctx);
        let title = match self.active.and_then(|i| self.tabs.get(i)) {
            Some(t) => format!("Gitr — {}", t.name),
            None => "Gitr".to_owned(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let p = theme::pal(ui);
        egui::Panel::top("toolbar")
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::top("tabbar")
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(8, 4)))
            .show(ui, |ui| self.tab_bar(ui));
        egui::Panel::bottom("statusbar")
            .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(8, 3)))
            .show(ui, |ui| self.status_bar(ui));
        if self.show_repo_list {
            egui::Panel::left("repo_list")
                .resizable(true)
                .default_size(200.0)
                .min_size(140.0)
                .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(8, 0)))
                .show(ui, |ui| self.repo_list(ui));
        }

        let mut open_repo = None;
        if let Some(tab) = self.active.and_then(|i| self.tabs.get_mut(i)) {
            let mut cx = Ctx { dialog: &mut self.dialog, settings: &mut self.settings, focus_search, focus_commit, open_repo: None };
            egui::Panel::left("repo_sidebar")
                .resizable(true)
                .default_size(250.0)
                .min_size(170.0)
                .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(8, 0)))
                .show(ui, |ui| sidebar::sidebar(ui, tab, &mut cx));
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(8, 6)))
                .show(ui, |ui| match tab.view {
                    View::History => {
                        let h = ui.available_height();
                        egui::Panel::bottom("details")
                            .resizable(true)
                            .default_size(h * 0.45)
                            .min_size(120.0)
                            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 6, bottom: 0 }))
                            .show(ui, |ui| history::details_panel(ui, tab, &mut cx));
                        egui::CentralPanel::no_frame().show(ui, |ui| history::commit_list(ui, tab, &mut cx));
                    }
                    View::LocalChanges => changes::changes_view(ui, tab, &mut cx),
                });
            open_repo = cx.open_repo.take();
        } else {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(p.panel)).show(ui, |ui| self.home(ui));
        }
        if let Some(path) = open_repo {
            self.open_repo(&ctx, &path);
        }

        if let Some(mut dialog) = self.dialog.take() {
            let tab = self.active.and_then(|i| self.tabs.get_mut(i));
            match dialogs::show(&ctx, &mut dialog, tab) {
                Outcome::Open => self.dialog = Some(dialog),
                Outcome::Close => {}
                Outcome::App(action) => self.handle_app_action(&ctx, action),
            }
        }
        if self.show_activity {
            self.activity_window(&ctx);
        }
        for tab in &mut self.tabs {
            crate::ui::file_history::show(&ctx, tab, self.settings.diff_font_size);
        }
        self.toasts(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let state = Persisted {
            repos: self.repos.clone(),
            open_tabs: self.tabs.iter().map(|t| t.path.clone()).collect(),
            active_tab: self.active,
            settings: self.settings.clone(),
            hide_repo_list: !self.show_repo_list,
        };
        eframe::set_value(storage, STORAGE_KEY, &state);
    }
}
