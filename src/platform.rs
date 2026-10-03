//! OS integration: file manager, terminal, default applications.

use std::path::Path;
use std::process::Command;

fn spawn(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd.spawn();
}

/// Shows a file or folder in the system file manager.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        if path.is_dir() {
            spawn(Command::new("open").arg(path));
        } else {
            spawn(Command::new("open").arg("-R").arg(path));
        }
    }
    #[cfg(windows)]
    {
        if path.is_dir() {
            spawn(Command::new("explorer").arg(path));
        } else {
            spawn(Command::new("explorer").arg(format!("/select,{}", path.display())));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        spawn(Command::new("xdg-open").arg(dir));
    }
}

/// Opens a file with its default application.
pub fn open_file(path: &Path) {
    #[cfg(target_os = "macos")]
    spawn(Command::new("open").arg(path));
    #[cfg(windows)]
    spawn(Command::new("cmd").args(["/C", "start", ""]).arg(path));
    #[cfg(all(unix, not(target_os = "macos")))]
    spawn(Command::new("xdg-open").arg(path));
}

/// Opens a terminal window in `dir`.
pub fn open_terminal(dir: &Path) {
    #[cfg(target_os = "macos")]
    spawn(Command::new("open").args(["-a", "Terminal"]).arg(dir));
    #[cfg(windows)]
    {
        // Prefer Windows Terminal, fall back to cmd.
        let ok = Command::new("wt").arg("-d").arg(dir).spawn().is_ok();
        if !ok {
            spawn(
                Command::new("cmd")
                    .args(["/C", "start", "cmd"])
                    .current_dir(dir),
            );
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for term in [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "xfce4-terminal",
            "alacritty",
            "kitty",
            "xterm",
        ] {
            if Command::new(term).current_dir(dir).spawn().is_ok() {
                return;
            }
        }
    }
}

pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    spawn(Command::new("open").arg(url));
    #[cfg(windows)]
    spawn(Command::new("cmd").args(["/C", "start", "", url]));
    #[cfg(all(unix, not(target_os = "macos")))]
    spawn(Command::new("xdg-open").arg(url));
}

/// Converts a remote URL (ssh or https) to a browsable https URL.
pub fn web_url(remote: &str) -> Option<String> {
    let r = remote.trim().trim_end_matches(".git");
    if r.starts_with("http://") || r.starts_with("https://") {
        // Strip credentials.
        let (scheme, rest) = r.split_once("://")?;
        let rest = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
        return Some(format!("{scheme}://{rest}"));
    }
    if let Some(rest) = r.strip_prefix("ssh://") {
        let rest = rest.split_once('@').map(|(_, h)| h).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        let host = host.split(':').next()?;
        return Some(format!("https://{host}/{path}"));
    }
    // scp-like: git@host:owner/repo
    let rest = r.split_once('@').map(|(_, h)| h).unwrap_or(r);
    let (host, path) = rest.split_once(':')?;
    Some(format!("https://{host}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::web_url;

    #[test]
    fn converts_remote_urls() {
        assert_eq!(
            web_url("git@github.com:a/b.git").as_deref(),
            Some("https://github.com/a/b")
        );
        assert_eq!(
            web_url("https://user@github.com/a/b.git").as_deref(),
            Some("https://github.com/a/b")
        );
        assert_eq!(
            web_url("ssh://git@host:22/a/b").as_deref(),
            Some("https://host/a/b")
        );
    }
}
