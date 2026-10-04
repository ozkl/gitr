//! GitHub accounts and their repositories.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{RichText, Sense, Vec2};
use egui_phosphor::regular as icon;

use crate::app::Settings;
use crate::github::{self, Account, Repo, Secret};
use crate::repo::Loaded;
use crate::ui::theme;

enum Msg {
    /// An account whose token was already in the credential store at startup.
    Existing(Option<Account>),
    SignedIn(Result<Account, String>),
    Repos(String, Result<Vec<Repo>, String>),
    SignedOut(String, Result<(), String>),
}

#[derive(PartialEq)]
enum Panel {
    /// Accounts on the left, the selected account's repositories on the right.
    Accounts,
    /// Adding an account.
    SignIn,
}

/// What the user asked for from the repository list.
pub enum Action {
    /// Open the clone dialog for this repository.
    Clone { url: String, name: String },
}

pub struct GitHub {
    ctx: egui::Context,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Logins of the signed-in accounts. Tokens live in the OS credential store.
    accounts: Vec<String>,
    /// The account list changed and must be written to settings.
    dirty: bool,
    selected: Option<String>,
    repos: HashMap<String, Loaded<Vec<Repo>>>,
    panel: Option<Panel>,
    token_input: String,
    busy: bool,
    error: Option<String>,
    filter: String,
}

impl GitHub {
    pub fn new(ctx: &egui::Context, settings: &Settings) -> Self {
        let (tx, rx) = channel();
        let accounts = settings.github_accounts.clone();
        github::set_accounts(accounts.clone());
        let gh = Self {
            ctx: ctx.clone(),
            tx,
            rx,
            selected: accounts.first().cloned(),
            accounts,
            dirty: false,
            repos: HashMap::new(),
            panel: None,
            token_input: String::new(),
            busy: false,
            error: None,
            filter: String::new(),
        };
        // Dev screenshots must not read the real credential store.
        #[cfg(feature = "screenshot")]
        if std::env::var_os("GITR_NO_GITHUB_CHECK").is_some() {
            return gh;
        }
        // No accounts yet: adopt a github.com token git already has in the credential store.
        if gh.accounts.is_empty() {
            gh.spawn(|| Msg::Existing(github::existing_account()));
        }
        gh
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    /// Fills the panel with made-up accounts and repositories (dev screenshots only).
    #[cfg(feature = "screenshot")]
    pub fn open_demo(&mut self) {
        let repo = |full: &str, desc: &str, private: bool, date: &str| Repo {
            full_name: full.to_owned(),
            description: Some(desc.to_owned()).filter(|d| !d.is_empty()),
            private,
            fork: false,
            archived: false,
            clone_url: format!("https://github.com/{full}.git"),
            ssh_url: format!("git@github.com:{full}.git"),
            html_url: format!("https://github.com/{full}"),
            pushed_at: Some(format!("{date}T10:00:00Z")),
        };
        self.accounts = vec!["ada".to_owned(), "ada-at-acme".to_owned()];
        self.selected = Some("ada".to_owned());
        self.repos.insert(
            "ada".to_owned(),
            Loaded::Ready(vec![
                repo(
                    "ada/analytical-engine",
                    "Programs for the Analytical Engine",
                    false,
                    "2026-10-01",
                ),
                repo("ada/notes", "", true, "2026-09-21"),
                repo(
                    "ada/bernoulli",
                    "Computing Bernoulli numbers",
                    false,
                    "2026-08-02",
                ),
            ]),
        );
        self.panel = Some(Panel::Accounts);
    }

    /// Label for the start-screen button.
    pub fn summary(&self) -> String {
        match self.accounts.as_slice() {
            [] => "Sign in to GitHub".to_owned(),
            [one] => format!("@{one}"),
            many => format!("GitHub ({} accounts)", many.len()),
        }
    }

    /// Opens the accounts panel.
    pub fn open(&mut self) {
        self.error = None;
        self.panel = Some(Panel::Accounts);
        self.ensure_repos();
    }

    fn add_account(&mut self, login: String) {
        if !self.accounts.iter().any(|a| a.eq_ignore_ascii_case(&login)) {
            self.accounts.push(login.clone());
            self.accounts_changed();
        }
        self.selected = Some(login);
    }

    fn accounts_changed(&mut self) {
        self.dirty = true;
        github::set_accounts(self.accounts.clone());
    }

    /// Loads the selected account's repositories if they have not been loaded yet.
    fn ensure_repos(&mut self) {
        if let Some(login) = self.selected.clone() {
            if !self.repos.contains_key(&login) {
                self.load_repos(login);
            }
        }
    }

    fn load_repos(&mut self, login: String) {
        self.repos.insert(login.clone(), Loaded::Loading);
        self.spawn(move || {
            let result = github::fetch_repos(&login);
            Msg::Repos(login, result)
        });
    }

    fn wipe_token_input(&mut self) {
        drop(Secret::new(std::mem::take(&mut self.token_input)));
    }

    fn close_sign_in(&mut self) {
        self.busy = false;
        self.wipe_token_input();
        // Back to the accounts panel.
        self.panel = Some(Panel::Accounts);
    }

    fn process_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Existing(Some(account)) => self.add_account(account.login),
                Msg::Existing(None) => {}
                Msg::SignedIn(result) => {
                    self.busy = false;
                    match result {
                        Ok(account) => {
                            self.error = None;
                            self.wipe_token_input();
                            self.repos.remove(&account.login);
                            self.add_account(account.login);
                            if self.panel.is_some() {
                                self.panel = Some(Panel::Accounts);
                                self.ensure_repos();
                            }
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Msg::Repos(login, result) => {
                    self.repos.insert(login, result.into());
                }
                Msg::SignedOut(login, result) => {
                    self.busy = false;
                    match result {
                        Ok(()) => {
                            self.accounts.retain(|a| *a != login);
                            self.repos.remove(&login);
                            self.accounts_changed();
                            self.selected = self.accounts.first().cloned();
                            self.ensure_repos();
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }

    fn start_token_sign_in(&mut self) {
        self.busy = true;
        self.error = None;
        let token = Secret::new(std::mem::take(&mut self.token_input).trim().to_owned());
        self.spawn(move || Msg::SignedIn(github::sign_in_with_token(token)));
    }

    /// Draws the open panel, if any.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Option<Action> {
        self.process_messages();
        if std::mem::take(&mut self.dirty) {
            settings.github_accounts = self.accounts.clone();
        }
        match self.panel {
            Some(Panel::SignIn) => {
                self.sign_in_panel(ctx);
                None
            }
            Some(Panel::Accounts) => self.accounts_panel(ctx, settings),
            None => None,
        }
    }

    fn sign_in_panel(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("github_sign_in")).show(ctx, |ui| {
            let p = theme::pal(ui);
            ui.set_width(460.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::GITHUB_LOGO).size(26.0));
                ui.heading(if self.accounts.is_empty() { "Sign in to GitHub" } else { "Add GitHub Account" });
            });
            ui.add_space(4.0);
            let note = format!(
                "{} Your access token is kept in {}. Gitr never saves it itself.",
                icon::LOCK_KEY,
                github::credential_store_name()
            );
            ui.add(egui::Label::new(RichText::new(note).small().color(p.muted)).wrap());
            ui.add_space(10.0);

            ui.label(RichText::new("Personal access token").strong());
            let hint = "Create a token with the \"repo\" scope and paste it here. The account is recognised from the token.";
            ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).wrap());
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut self.token_input)
                    .password(true)
                    .hint_text("ghp_… or github_pat_…")
                    .desired_width(300.0);
                ui.add_enabled(!self.busy, edit);
                let can = !self.busy && !self.token_input.trim().is_empty();
                if ui.add_enabled(can, egui::Button::new("Sign In")).clicked() {
                    self.start_token_sign_in();
                }
            });
            if ui.link(format!("{} Create a token on GitHub", icon::ARROW_SQUARE_OUT)).clicked() {
                crate::platform::open_url(github::NEW_TOKEN_URL);
            }
            if self.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Contacting GitHub…").color(p.muted));
                });
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(e).color(p.removed)).wrap());
            }
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close || modal.should_close() {
            self.close_sign_in();
        }
    }

    fn accounts_panel(&mut self, ctx: &egui::Context, settings: &mut Settings) -> Option<Action> {
        let mut action = None;
        let mut close = false;
        let mut sign_out = None;
        let mut refresh = false;
        let mut add = false;
        let mut select = None;
        const HEIGHT: f32 = 440.0;
        let modal = egui::Modal::new(egui::Id::new("github_accounts")).show(ctx, |ui| {
            let p = theme::pal(ui);
            ui.set_width(860.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::GITHUB_LOGO).size(24.0));
                ui.label(RichText::new("GitHub").size(17.0).strong());
            });
            ui.add_space(6.0);
            let row = Vec2::new(860.0, HEIGHT + 64.0);
            let layout = egui::Layout::left_to_right(egui::Align::Min);
            ui.allocate_ui_with_layout(row, layout, |ui| {
                // Keep the row (and its divider) from stretching to the window height.
                ui.set_max_height(row.y);
                // Accounts column.
                ui.allocate_ui(Vec2::new(210.0, HEIGHT + 64.0), |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(210.0);
                        ui.label(RichText::new("ACCOUNTS").small().color(p.muted));
                        ui.add_space(2.0);
                        for login in &self.accounts {
                            let is_sel = self.selected.as_deref() == Some(login.as_str());
                            let (rect, resp) = ui.allocate_exact_size(Vec2::new(210.0, 30.0), Sense::click());
                            if is_sel {
                                ui.painter().rect_filled(rect, 5.0, p.selection);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 5.0, p.hover);
                            }
                            let color = if is_sel { p.selection_text } else { ui.visuals().text_color() };
                            let avatar = egui::pos2(rect.left() + 16.0, rect.center().y);
                            ui.painter().circle_filled(avatar, 9.0, theme::hash_color(login));
                            ui.painter().text(avatar, egui::Align2::CENTER_CENTER, theme::initials(login), egui::FontId::proportional(9.0), egui::Color32::WHITE);
                            ui.painter().with_clip_rect(rect).text(
                                egui::pos2(rect.left() + 32.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                format!("@{login}"),
                                egui::FontId::proportional(13.5),
                                color,
                            );
                            if resp.clicked() {
                                select = Some(login.clone());
                            }
                            resp.context_menu(|ui| {
                                if ui.button(format!("{} Open Profile", icon::ARROW_SQUARE_OUT)).clicked() {
                                    crate::platform::open_url(&format!("https://github.com/{login}"));
                                    ui.close();
                                }
                                if ui.button(format!("{} Sign Out", icon::SIGN_OUT)).clicked() {
                                    sign_out = Some(login.clone());
                                    ui.close();
                                }
                            });
                        }
                        ui.add_space(6.0);
                        if ui.button(format!("{} Add Account…", icon::PLUS)).clicked() {
                            add = true;
                        }
                        if let Some(login) = self.selected.clone() {
                            let hint = format!("Removes @{login}'s token from {}", github::credential_store_name());
                            if ui.add_enabled(!self.busy, egui::Button::new(format!("{} Sign Out @{login}", icon::SIGN_OUT))).on_hover_text(hint).clicked() {
                                sign_out = Some(login);
                            }
                        }
                        ui.add_space(8.0);
                        let note = "Repositories with an HTTPS GitHub remote use the matching account automatically.";
                        ui.add(egui::Label::new(RichText::new(note).small().color(p.muted)).wrap());
                    });
                });
                ui.separator();

                // Repositories of the selected account.
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let hint = format!("{} Filter repositories", icon::MAGNIFYING_GLASS);
                        ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(hint).desired_width(280.0));
                        if ui.button(icon::ARROWS_CLOCKWISE).on_hover_text("Refresh").clicked() {
                            refresh = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.selectable_value(&mut settings.github_clone_via_ssh, true, "SSH");
                            ui.selectable_value(&mut settings.github_clone_via_ssh, false, "HTTPS");
                            ui.label(RichText::new("Clone with").color(p.muted));
                        });
                    });
                    ui.add_space(4.0);
                    let repos = self.selected.as_ref().and_then(|l| self.repos.get(l));
                    match repos {
                        None if self.accounts.is_empty() => {
                            ui.allocate_ui(Vec2::new(ui.available_width(), HEIGHT), |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(HEIGHT * 0.3);
                                    ui.label(RichText::new(icon::GITHUB_LOGO).size(40.0).color(p.muted));
                                    ui.label(RichText::new("No accounts yet").size(16.0).strong());
                                    ui.label(RichText::new("Add a GitHub account to browse and clone its repositories.").color(p.muted));
                                    ui.add_space(8.0);
                                    if ui.button(format!("{} Add Account…", icon::PLUS)).clicked() {
                                        add = true;
                                    }
                                });
                            });
                        }
                        Some(Loaded::Ready(repos)) => {
                            let q = self.filter.trim().to_lowercase();
                            let shown: Vec<&Repo> = repos
                                .iter()
                                .filter(|r| {
                                    q.is_empty()
                                        || r.full_name.to_lowercase().contains(&q)
                                        || r.description.as_deref().unwrap_or_default().to_lowercase().contains(&q)
                                })
                                .collect();
                            ui.spacing_mut().item_spacing.y = 0.0;
                            egui::ScrollArea::vertical().max_height(HEIGHT).auto_shrink([false, false]).show_rows(ui, 46.0, shown.len(), |ui, range| {
                                for repo in &shown[range] {
                                    if let Some(a) = repo_row(ui, repo, settings.github_clone_via_ssh) {
                                        action = Some(a);
                                    }
                                }
                            });
                            ui.add_space(4.0);
                            ui.label(RichText::new(format!("{} of {} repositories", shown.len(), repos.len())).small().color(p.muted));
                        }
                        Some(Loaded::Failed(e)) => {
                            ui.allocate_ui(Vec2::new(ui.available_width(), HEIGHT), |ui| {
                                ui.add(egui::Label::new(RichText::new(e).color(p.removed)).wrap());
                            });
                        }
                        _ => {
                            ui.allocate_ui(Vec2::new(ui.available_width(), HEIGHT), |ui| {
                                ui.centered_and_justified(|ui| ui.spinner());
                            });
                        }
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.add(egui::Label::new(RichText::new(e).color(p.removed)).wrap());
            }
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });

        if let Some(login) = select {
            self.selected = Some(login);
            self.ensure_repos();
        }
        if refresh {
            if let Some(login) = self.selected.clone() {
                self.load_repos(login);
            }
        }
        if add {
            self.error = None;
            self.panel = Some(Panel::SignIn);
        } else if close || modal.should_close() || action.is_some() {
            self.panel = None;
        }
        if let Some(login) = sign_out {
            self.busy = true;
            self.error = None;
            self.spawn(move || {
                let result = github::forget_token(&login);
                Msg::SignedOut(login, result)
            });
        }
        action
    }
}

fn repo_row(ui: &mut egui::Ui, repo: &Repo, ssh: bool) -> Option<Action> {
    let p = theme::pal(ui);
    let mut action = None;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::hover());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 5.0, p.hover);
    }
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 4.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.label(
        RichText::new(if repo.private {
            icon::LOCK
        } else {
            icon::BOOK_BOOKMARK
        })
        .size(17.0)
        .color(p.muted),
    )
    .on_hover_text(if repo.private { "Private" } else { "Public" });
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button("Clone").clicked() {
            let url = if ssh {
                repo.ssh_url.clone()
            } else {
                repo.clone_url.clone()
            };
            let name = repo
                .full_name
                .rsplit('/')
                .next()
                .unwrap_or(&repo.full_name)
                .to_owned();
            action = Some(Action::Clone { url, name });
        }
        if ui
            .button(icon::ARROW_SQUARE_OUT)
            .on_hover_text("Open on GitHub")
            .clicked()
        {
            crate::platform::open_url(&repo.html_url);
        }
        if let Some(date) = repo.pushed_at.as_deref().and_then(|d| d.get(..10)) {
            ui.label(RichText::new(date).small().color(p.muted))
                .on_hover_text("Last push");
        }
        // Name and description fill the remaining width.
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&repo.full_name).strong()).truncate());
                for (flag, text) in [(repo.fork, "fork"), (repo.archived, "archived")] {
                    if flag {
                        ui.label(RichText::new(text).small().color(p.muted));
                    }
                }
            });
            let desc = repo.description.as_deref().unwrap_or_default();
            ui.add(egui::Label::new(RichText::new(desc).small().color(p.muted)).truncate());
        });
    });
    action
}
