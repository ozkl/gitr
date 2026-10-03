//! Credential prompts for git and ssh.
//!
//! git (`GIT_ASKPASS`) and ssh (`SSH_ASKPASS`) run this executable with the prompt as the
//! first argument whenever they need a username, password, passphrase or host-key
//! confirmation. We show a small dialog and print the answer to stdout, which goes straight
//! back to git/ssh through a pipe. Nothing is logged or written to disk, and the answer is
//! wiped from memory after it has been handed over.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use egui::{Key, RichText};

/// Environment variable marking a process started by git/ssh as an askpass request.
pub const MODE_ENV: &str = "GITR_ASKPASS_MODE";

/// True when this process was launched by git/ssh to ask for credentials.
pub fn is_askpass_invocation() -> bool {
    std::env::var_os(MODE_ENV).is_some()
}

/// Environment for git commands so that prompts are routed to this executable.
pub fn env_for_git(exe: &Path) -> Vec<(&'static str, String)> {
    let exe = exe.display().to_string();
    let mut env = vec![
        (MODE_ENV, "1".to_owned()),
        ("GIT_ASKPASS", exe.clone()),
        ("SSH_ASKPASS", exe),
        // OpenSSH ≥ 8.4: use askpass even though there is no terminal check to fail.
        ("SSH_ASKPASS_REQUIRE", "force".to_owned()),
    ];
    // Older OpenSSH only uses SSH_ASKPASS when DISPLAY is set.
    if std::env::var_os("DISPLAY").is_none() {
        env.push(("DISPLAY", ":0".to_owned()));
    }
    env
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Password, passphrase, token, PIN: masked input.
    Secret,
    /// Username and other plain answers.
    Text,
    /// ssh host-key confirmation ("yes/no/[fingerprint]").
    YesNo,
}

fn classify(prompt: &str) -> Kind {
    let p = prompt.to_lowercase();
    if p.contains("(yes/no") {
        Kind::YesNo
    } else if ["password", "passphrase", "token", "pin", "passcode", "secret"].iter().any(|w| p.contains(w)) {
        Kind::Secret
    } else {
        Kind::Text
    }
}

struct Dialog {
    prompt: String,
    kind: Kind,
    input: String,
    show_secret: bool,
    result: Rc<RefCell<Option<String>>>,
    focused: bool,
    #[cfg(feature = "screenshot")]
    devshot: crate::devshot::DevShot,
}

impl Dialog {
    fn finish(&mut self, ui: &egui::Ui, answer: Option<String>) {
        *self.result.borrow_mut() = answer;
        wipe(&mut self.input);
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Dialog {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(feature = "screenshot")]
        self.devshot.tick(ui.ctx());
        if !self.focused {
            self.focused = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let glyph = match self.kind {
                    Kind::YesNo => egui_phosphor::regular::SHIELD_WARNING,
                    _ => egui_phosphor::regular::LOCK_KEY,
                };
                ui.label(RichText::new(glyph).size(28.0));
                ui.vertical(|ui| {
                    ui.label(RichText::new("Git needs your input").strong().size(15.0));
                    ui.add(egui::Label::new(self.prompt.trim()).wrap());
                });
            });
            ui.add_space(8.0);

            let mut submit = false;
            if self.kind != Kind::YesNo {
                let secret = self.kind == Kind::Secret && !self.show_secret;
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .password(secret)
                        .desired_width(f32::INFINITY),
                );
                if !r.has_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                submit = ui.input(|i| i.key_pressed(Key::Enter));
                if self.kind == Kind::Secret {
                    ui.checkbox(&mut self.show_secret, "Show");
                }
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Gitr passes your answer directly to git and does not store it. \
                     Git may save working credentials in its configured credential helper (e.g. the OS keychain).",
                )
                .small()
                .weak(),
            );

            ui.add_space(8.0);
            let cancel = ui.input(|i| i.key_pressed(Key::Escape));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.kind == Kind::YesNo {
                    if ui.button("Trust and Connect").clicked() || submit {
                        self.finish(ui, Some("yes".to_owned()));
                    }
                    if ui.button("Cancel").clicked() || cancel {
                        self.finish(ui, Some("no".to_owned()));
                    }
                } else {
                    if ui.button("OK").clicked() || submit {
                        let answer = self.input.clone();
                        self.finish(ui, Some(answer));
                    }
                    if ui.button("Cancel").clicked() || cancel {
                        self.finish(ui, None);
                    }
                }
            });
        });
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// Overwrites the string's bytes before freeing it.
fn wipe(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    bytes.fill(0);
    std::hint::black_box(&bytes);
}

/// Runs the prompt dialog and returns the process exit code.
pub fn run(prompt: String) -> i32 {
    let kind = classify(&prompt);
    let result = Rc::new(RefCell::new(None));
    let height = if kind == Kind::YesNo { 230.0 } else { 220.0 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gitr — Authentication")
            .with_inner_size([480.0, height])
            .with_resizable(false)
            .with_always_on_top(),
        centered: true,
        persist_window: false,
        // Separate from the main app's state (whose saved window size would otherwise apply).
        // Nothing is ever written here: window and UI memory persistence are both off.
        persistence_path: Some(std::env::temp_dir().join("gitr-askpass.ron")),
        ..Default::default()
    };
    let app_result = result.clone();
    let run = eframe::run_native(
        "gitr-askpass",
        options,
        Box::new(move |cc| {
            crate::ui::theme::install_fonts(&cc.egui_ctx);
            crate::ui::theme::apply_style(&cc.egui_ctx);
            Ok(Box::new(Dialog {
                prompt,
                kind,
                input: String::new(),
                show_secret: false,
                result: app_result,
                focused: false,
                #[cfg(feature = "screenshot")]
                devshot: crate::devshot::DevShot::from_env(),
            }))
        }),
    );
    if run.is_err() {
        return 1;
    }
    let answer = result.borrow_mut().take();
    match answer {
        Some(mut answer) => {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let ok = writeln!(out, "{answer}").and_then(|_| out.flush()).is_ok();
            wipe(&mut answer);
            if ok { 0 } else { 1 }
        }
        // Cancelled: a non-zero exit makes git/ssh abort the operation.
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_prompts() {
        assert_eq!(classify("Username for 'https://github.com': "), Kind::Text);
        assert_eq!(classify("Password for 'https://alp@github.com': "), Kind::Secret);
        assert_eq!(classify("Enter passphrase for key '/Users/alp/.ssh/id_ed25519': "), Kind::Secret);
        assert_eq!(
            classify("Are you sure you want to continue connecting (yes/no/[fingerprint])? "),
            Kind::YesNo
        );
    }

    #[test]
    fn wipe_clears_string() {
        let mut s = String::from("hunter2");
        wipe(&mut s);
        assert!(s.is_empty());
    }
}
