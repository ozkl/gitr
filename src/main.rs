#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod askpass;
#[cfg(feature = "screenshot")]
mod devshot;
mod format;
mod git;
mod github;
mod platform;
mod preview;
mod repo;
mod ui;
mod workspace;

/// App icon (rendered from `assets/logo.svg`).
pub const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

/// Apps started from Finder/Dock get a minimal PATH; add the usual Homebrew locations so that
/// git, git-lfs, credential helpers and hooks installed there are found.
#[cfg(target_os = "macos")]
fn extend_path() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<std::path::PathBuf> = std::env::split_paths(&current).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let extra = std::path::PathBuf::from(extra);
        if extra.is_dir() && !paths.contains(&extra) {
            paths.push(extra);
        }
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        // SAFETY: called at startup before any other threads exist.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

fn main() -> eframe::Result {
    #[cfg(target_os = "macos")]
    extend_path();

    // Launched by git/ssh to ask for credentials: show the prompt dialog only.
    if askpass::is_askpass_invocation() {
        let prompt = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "Password:".to_owned());
        std::process::exit(askpass::run(prompt));
    }
    git::cmd::set_config_hook(github::credential_config);
    if let Ok(exe) = std::env::current_exe() {
        git::cmd::set_extra_env(askpass::env_for_git(&exe));
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("gitr")
            .with_title("Gitr")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_drag_and_drop(true)
            .with_icon(eframe::icon_data::from_png_bytes(ICON_PNG).unwrap_or_default()),
        ..Default::default()
    };
    // Keep the window visible while capturing (occluded windows are not rendered).
    #[cfg(feature = "screenshot")]
    let options = if std::env::var("GITR_SCREENSHOT").is_ok() {
        eframe::NativeOptions {
            viewport: options.viewport.with_always_on_top(),
            ..options
        }
    } else {
        options
    };
    // "gitr" is the storage id (settings folder); the visible name is "Gitr".
    eframe::run_native(
        "gitr",
        options,
        Box::new(|cc| Ok(Box::new(app::GitrApp::new(cc)))),
    )
}
