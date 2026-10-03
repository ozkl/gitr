//! Thin wrapper around the `git` executable.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

static GIT_BINARY: OnceLock<String> = OnceLock::new();
static EXTRA_ENV: OnceLock<Vec<(&'static str, String)>> = OnceLock::new();

/// Environment added to every git command (used to route credential prompts to our dialog).
pub fn set_extra_env(env: Vec<(&'static str, String)>) {
    let _ = EXTRA_ENV.set(env);
}

/// Overrides the git executable (defaults to `git` on `PATH`). Only the first call has effect.
pub fn set_git_binary(path: &str) {
    if !path.trim().is_empty() {
        let _ = GIT_BINARY.set(path.trim().to_owned());
    }
}

fn git_binary() -> &'static str {
    GIT_BINARY.get().map(String::as_str).unwrap_or("git")
}

#[derive(Debug, Clone)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
}

fn base_command(repo: &Path) -> Command {
    let mut cmd = Command::new(git_binary());
    cmd.current_dir(repo)
        .args([
            "-c",
            "core.quotepath=false",
            "-c",
            "color.ui=false",
            "-c",
            "log.showSignature=false",
            "--no-pager",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .envs(EXTRA_ENV.get().into_iter().flatten().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn describe(args: &[&str]) -> String {
    format!("git {}", args.join(" "))
}

/// Runs git and returns stdout on success, or a readable error message on failure.
pub fn run(repo: &Path, args: &[&str]) -> Result<String, String> {
    run_full(repo, args).map(|o| o.stdout)
}

pub fn run_full(repo: &Path, args: &[&str]) -> Result<GitOutput, String> {
    let output = base_command(repo)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {}: {e}", describe(args)))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok(GitOutput { stdout, stderr })
    } else {
        let mut msg = stderr.trim().to_owned();
        if msg.is_empty() {
            msg = stdout.trim().to_owned();
        }
        if msg.is_empty() {
            msg = format!("{} exited with {}", describe(args), output.status);
        }
        Err(msg)
    }
}

/// Runs git feeding `input` on stdin (used for `git apply`).
pub fn run_with_input(repo: &Path, args: &[&str], input: &[u8]) -> Result<String, String> {
    run_with_input_bytes(repo, args, input).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Like [`run_with_input`], returning raw stdout. Input is written from a separate thread so
/// that commands producing output while reading (check-attr, lfs smudge) cannot deadlock.
pub fn run_with_input_bytes(repo: &Path, args: &[&str], input: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = base_command(repo)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run {}: {e}", describe(args)))?;
    let writer = child.stdin.take().map(|mut stdin| {
        let input = input.to_vec();
        std::thread::spawn(move || stdin.write_all(&input))
    });
    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for git: {e}"))?;
    if let Some(writer) = writer {
        // A failed write shows up as a git error below; nothing more to report here.
        let _ = writer.join();
    }
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Raw bytes of stdout (for file contents that may be binary).
pub fn run_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = base_command(repo)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {}: {e}", describe(args)))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// `git --version` output, computed once.
pub fn git_version() -> Option<String> {
    static VERSION: OnceLock<Option<String>> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            let out = Command::new(git_binary()).arg("--version").output().ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        })
        .clone()
}
