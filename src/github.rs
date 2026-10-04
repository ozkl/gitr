//! GitHub account support: sign-in, the account's repositories.
//!
//! Gitr never stores the access token. It is handed to git's credential helper
//! (`git credential approve`), which keeps it in the operating system's secure store
//! (the macOS Keychain via `osxkeychain`, Git Credential Manager on Windows, libsecret on
//! Linux), and read back on demand with `git credential fill` for the duration of a request.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, RwLock};
use std::time::Duration;

use serde::Deserialize;

const HOST: &str = "github.com";
const API: &str = "https://api.github.com";
/// Page for creating a personal access token with the right scope.
pub const NEW_TOKEN_URL: &str =
    "https://github.com/settings/tokens/new?scopes=repo&description=Gitr";

/// A secret that is overwritten when dropped and never printed.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Repo {
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub private: bool,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub archived: bool,
    pub clone_url: String,
    pub ssh_url: String,
    pub html_url: String,
    #[serde(default)]
    pub pushed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Credential storage (delegated to git's credential helper)

/// Runs `git credential <action>` without Gitr's askpass, so a missing credential fails
/// instead of opening a prompt.
fn git_credential(action: &str, input: &str) -> Result<String, String> {
    // On macOS make sure the Keychain helper is used even if no helper is configured.
    #[cfg(target_os = "macos")]
    if !has_credential_helper() {
        return git_credential_with(&["-c", "credential.helper=osxkeychain"], action, input);
    }
    git_credential_with(&[], action, input)
}

/// `config` is extra `-c key=value` arguments placed before the subcommand.
fn git_credential_with(config: &[&str], action: &str, input: &str) -> Result<String, String> {
    use std::io::Write;
    let mut cmd = Command::new(crate::git::cmd::git_binary());
    cmd.args(config);
    cmd.args(["-c", "core.askPass=", "credential", action])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to run git credential: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // The helper reads everything before answering; input is tiny.
        let _ = stdin.write_all(input.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Whether git has a credential helper configured (where the token will be kept).
pub fn has_credential_helper() -> bool {
    Command::new(crate::git::cmd::git_binary())
        .args(["config", "--get-all", "credential.helper"])
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false)
}

/// Where the token ends up, for display.
pub fn credential_store_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "the macOS Keychain"
    } else if cfg!(windows) {
        "Windows Credential Manager"
    } else if has_credential_helper() {
        "your git credential helper"
    } else {
        "memory only (no git credential helper is configured)"
    }
}

fn credential_request(username: Option<&str>) -> String {
    let mut s = format!("protocol=https\nhost={HOST}\n");
    if let Some(u) = username {
        s.push_str(&format!("username={u}\n"));
    }
    s.push('\n');
    s
}

/// The stored GitHub token for `login` (or whichever one the store returns for `None`).
pub fn stored_token(login: Option<&str>) -> Option<Secret> {
    let out = git_credential("fill", &credential_request(login)).ok()?;
    let token = out
        .lines()
        .find_map(|l| l.strip_prefix("password="))?
        .to_owned();
    (!token.is_empty()).then(|| Secret::new(token))
}

/// Hands the token to the OS credential store.
fn store_token(login: &str, token: &Secret) -> Result<(), String> {
    let input = format!(
        "protocol=https\nhost={HOST}\nusername={login}\npassword={}\n\n",
        token.expose()
    );
    let result = git_credential("approve", &input).map(|_| ());
    // The request string holds a copy of the token; wipe it.
    drop(Secret::new(input));
    result
}

/// Removes the token from the OS credential store.
pub fn forget_token(login: &str) -> Result<(), String> {
    let Some(token) = stored_token(Some(login)) else {
        return Ok(());
    };
    let input = format!(
        "protocol=https\nhost={HOST}\nusername={login}\npassword={}\n\n",
        token.expose()
    );
    let result = git_credential("reject", &input).map(|_| ());
    drop(Secret::new(input));
    result
}

// ---------------------------------------------------------------------------
// HTTP

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("Gitr/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn api_get<T: serde::de::DeserializeOwned>(path: &str, token: &Secret) -> Result<T, String> {
    let mut resp = agent()
        .get(format!("{API}{path}"))
        .header("Authorization", format!("Bearer {}", token.expose()))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| format!("Could not reach GitHub: {e}"))?;
    match resp.status().as_u16() {
        200 => resp
            .body_mut()
            .read_json()
            .map_err(|e| format!("Unexpected response from GitHub: {e}")),
        401 => Err("GitHub rejected the token (expired or revoked). Sign in again.".to_owned()),
        403 => Err(
            "GitHub refused the request (rate limit, or the token lacks permission).".to_owned(),
        ),
        code => Err(format!("GitHub returned HTTP {code}")),
    }
}

pub fn fetch_account(token: &Secret) -> Result<Account, String> {
    api_get("/user", token)
}

/// An account whose token is already in the credential store (e.g. saved by git earlier).
pub fn existing_account() -> Option<Account> {
    fetch_account(&stored_token(None)?).ok()
}

/// Repositories `login` can access, most recently pushed first.
pub fn fetch_repos(login: &str) -> Result<Vec<Repo>, String> {
    let token = stored_token(Some(login))
        .ok_or_else(|| format!("No token for @{login} in the credential store. Sign in again."))?;
    let mut all = Vec::new();
    // 100 per page; stop at 2000 repositories.
    for page in 1..=20 {
        let batch: Vec<Repo> = api_get(
            &format!("/user/repos?per_page=100&sort=pushed&page={page}"),
            &token,
        )?;
        let done = batch.len() < 100;
        all.extend(batch);
        if done {
            break;
        }
    }
    Ok(all)
}

/// Verifies `token`, then stores it in the OS credential store.
pub fn sign_in_with_token(token: Secret) -> Result<Account, String> {
    let account = fetch_account(&token)?;
    store_token(&account.login, &token)?;
    Ok(account)
}

// ---------------------------------------------------------------------------
// Choosing the account for a repository

/// Logins of the signed-in accounts (tokens stay in the credential store).
static ACCOUNTS: RwLock<Vec<String>> = RwLock::new(Vec::new());
/// "owner/repo" -> account chosen for it this session.
static RESOLVED: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// Logins of the signed-in accounts.
pub fn accounts() -> Vec<String> {
    ACCOUNTS.read().unwrap().clone()
}

pub fn set_accounts(logins: Vec<String>) {
    *ACCOUNTS.write().unwrap() = logins;
    // Choices may change when accounts are added or removed.
    *RESOLVED.lock().unwrap() = None;
}

/// `(owner, repo)` of an HTTPS github.com URL without embedded credentials.
/// SSH URLs authenticate by key and are not handled here.
pub fn github_https_repo(url: &str) -> Option<(String, String)> {
    let rest = url.trim().strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    if !host.eq_ignore_ascii_case(HOST) {
        // Includes "user@github.com": the URL already names its account.
        return None;
    }
    let mut parts = path.trim_end_matches('/').splitn(2, '/');
    let owner = parts.next()?.to_owned();
    let repo = parts.next()?.trim_end_matches(".git").to_owned();
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/')).then_some((owner, repo))
}

/// What an account may do with a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    None,
    Read,
    Push,
}

/// Picks the account for a repository: the owner's own account, otherwise the account with
/// the most access (`probe` asks GitHub; it is only called when needed).
pub fn pick_account(
    accounts: &[String],
    owner: &str,
    probe: &dyn Fn(&str) -> Access,
) -> Option<String> {
    match accounts {
        [] => None,
        [only] => Some(only.clone()),
        _ => {
            if let Some(own) = accounts.iter().find(|a| a.eq_ignore_ascii_case(owner)) {
                return Some(own.clone());
            }
            let mut readable = None;
            for account in accounts {
                match probe(account) {
                    Access::Push => return Some(account.clone()),
                    Access::Read if readable.is_none() => readable = Some(account.clone()),
                    _ => {}
                }
            }
            readable
        }
    }
}

#[derive(Deserialize)]
struct RepoAccess {
    #[serde(default)]
    permissions: Option<Permissions>,
}

#[derive(Deserialize)]
struct Permissions {
    #[serde(default)]
    push: bool,
}

fn probe_access(login: &str, owner: &str, repo: &str) -> Access {
    let Some(token) = stored_token(Some(login)) else {
        return Access::None;
    };
    match api_get::<RepoAccess>(&format!("/repos/{owner}/{repo}"), &token) {
        Ok(r) if r.permissions.as_ref().is_some_and(|p| p.push) => Access::Push,
        Ok(_) => Access::Read,
        Err(_) => Access::None,
    }
}

/// The signed-in account to use for `owner/repo` (may ask GitHub once; cached afterwards).
pub fn account_for(owner: &str, repo: &str) -> Option<String> {
    let key = format!("{owner}/{repo}").to_lowercase();
    if let Some(hit) = RESOLVED
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&key).cloned())
    {
        return hit;
    }
    let accounts = ACCOUNTS.read().unwrap().clone();
    let choice = pick_account(&accounts, owner, &|login| probe_access(login, owner, repo));
    RESOLVED
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, choice.clone());
    choice
}

/// The account already chosen for a remote URL this session, without any network access.
pub fn cached_account_for_url(url: &str) -> Option<String> {
    let (owner, repo) = github_https_repo(url)?;
    let key = format!("{owner}/{repo}").to_lowercase();
    RESOLVED
        .lock()
        .unwrap()
        .as_ref()?
        .get(&key)
        .cloned()
        .flatten()
}

/// Git subcommands that talk to a remote.
fn is_network_command(args: &[&str]) -> bool {
    matches!(
        args.first().copied(),
        Some("fetch" | "pull" | "push" | "clone" | "ls-remote" | "submodule")
    )
}

fn plain_git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new(crate::git::cmd::git_binary())
        .current_dir(repo)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Extra `-c` options for a git command so that each GitHub HTTPS remote authenticates as
/// the right signed-in account. Empty when there is nothing to add.
pub fn credential_config(repo: &Path, args: &[&str]) -> Vec<String> {
    if !is_network_command(args) || ACCOUNTS.read().unwrap().is_empty() {
        return Vec::new();
    }
    // URLs involved: the ones named on the command line (clone), plus the repo's remotes.
    let mut urls: Vec<String> = args
        .iter()
        .filter(|a| a.starts_with("https://"))
        .map(|a| a.to_string())
        .collect();
    if let Some(out) = plain_git(repo, &["config", "--get-regexp", r"^remote\..*\.url$"]) {
        urls.extend(
            out.lines()
                .filter_map(|l| l.split_whitespace().nth(1))
                .map(str::to_owned),
        );
    }
    urls.sort();
    urls.dedup();
    let mut config = Vec::new();
    for url in urls {
        let Some((owner, name)) = github_https_repo(&url) else {
            continue;
        };
        // Respect an account the user configured for this URL themselves.
        if plain_git(
            repo,
            &["config", "--get-urlmatch", "credential.username", &url],
        )
        .is_some_and(|v| !v.trim().is_empty())
        {
            continue;
        }
        if let Some(login) = account_for(&owner, &name) {
            config.push("-c".to_owned());
            config.push(format!("credential.{url}.username={login}"));
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_not_printed() {
        let s = Secret::new("ghp_supersecret".into());
        assert_eq!(format!("{s:?}"), "Secret(***)");
    }

    #[test]
    fn credential_request_format() {
        assert_eq!(
            credential_request(None),
            "protocol=https\nhost=github.com\n\n"
        );
        assert_eq!(
            credential_request(Some("ada")),
            "protocol=https\nhost=github.com\nusername=ada\n\n"
        );
    }

    #[test]
    fn parses_repo_json() {
        let json = r#"[{"full_name":"ozkl/gitr","description":null,"private":true,"fork":false,"archived":false,
            "clone_url":"https://github.com/ozkl/gitr.git","ssh_url":"git@github.com:ozkl/gitr.git",
            "html_url":"https://github.com/ozkl/gitr","pushed_at":"2026-10-03T19:00:00Z","extra":1}]"#;
        let repos: Vec<Repo> = serde_json::from_str(json).unwrap();
        assert_eq!(repos[0].full_name, "ozkl/gitr");
        assert!(repos[0].private);
        assert_eq!(repos[0].ssh_url, "git@github.com:ozkl/gitr.git");
    }

    /// Round trip through git's credential machinery using an isolated file store, so the
    /// real Keychain is never touched.
    #[test]
    fn credential_helper_round_trip() {
        let file = std::env::temp_dir().join(format!("gitr-cred-{}", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let helper = format!("credential.helper=store --file={}", file.display());
        // An empty value first clears helpers from the user's config.
        let config = ["-c", "credential.helper=", "-c", helper.as_str()];

        // Nothing stored: fill must fail rather than prompt.
        assert!(git_credential_with(&config, "fill", &credential_request(None)).is_err());

        let input = "protocol=https\nhost=github.com\nusername=ada\npassword=fake-token-123\n\n";
        git_credential_with(&config, "approve", input).unwrap();
        let out = git_credential_with(&config, "fill", &credential_request(None)).unwrap();
        assert!(out.lines().any(|l| l == "password=fake-token-123"));
        assert!(out.lines().any(|l| l == "username=ada"));

        git_credential_with(&config, "reject", input).unwrap();
        assert!(git_credential_with(&config, "fill", &credential_request(None)).is_err());
        let _ = std::fs::remove_file(&file);
    }

    /// Talks to github.com with bogus values; run with `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs network"]
    fn network_rejects_bogus_token() {
        let err = fetch_account(&Secret::new("not-a-real-token".into())).unwrap_err();
        assert!(err.contains("rejected the token"), "{err}");
    }

    #[test]
    fn parses_github_https_urls() {
        let repo = |u| github_https_repo(u).map(|(o, r)| format!("{o}/{r}"));
        assert_eq!(
            repo("https://github.com/ozkl/gitr.git").as_deref(),
            Some("ozkl/gitr")
        );
        assert_eq!(
            repo("https://GitHub.com/Org/Repo/").as_deref(),
            Some("Org/Repo")
        );
        // Already names an account, SSH, and other hosts are left alone.
        assert_eq!(repo("https://ada@github.com/ozkl/gitr.git"), None);
        assert_eq!(repo("git@github.com:ozkl/gitr.git"), None);
        assert_eq!(repo("https://gitlab.com/a/b.git"), None);
        assert_eq!(repo("https://github.com/only-owner"), None);
    }

    #[test]
    fn picks_the_right_account() {
        let accounts = vec!["ozkl".to_owned(), "work-alp".to_owned()];
        let no_probe = |_: &str| -> Access { panic!("must not ask GitHub") };
        // One account: always that one. Owner match: no need to ask.
        assert_eq!(
            pick_account(&accounts[..1], "someone", &no_probe).as_deref(),
            Some("ozkl")
        );
        assert_eq!(
            pick_account(&accounts, "OZKL", &no_probe).as_deref(),
            Some("ozkl")
        );
        assert_eq!(pick_account(&[], "ozkl", &no_probe), None);
        // Organisation repo: the account with push access wins over read-only.
        let probe = |login: &str| {
            if login == "work-alp" {
                Access::Push
            } else {
                Access::Read
            }
        };
        assert_eq!(
            pick_account(&accounts, "acme", &probe).as_deref(),
            Some("work-alp")
        );
        let read_only = |login: &str| {
            if login == "ozkl" {
                Access::Read
            } else {
                Access::None
            }
        };
        assert_eq!(
            pick_account(&accounts, "acme", &read_only).as_deref(),
            Some("ozkl")
        );
        assert_eq!(pick_account(&accounts, "acme", &|_| Access::None), None);
    }

    #[test]
    fn credential_config_targets_each_github_remote() {
        let dir = std::env::temp_dir().join(format!("gitr-ghcfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| plain_git(&dir, args).unwrap();
        git(&["init", "-q"]);
        git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/ozkl/gitr.git",
        ]);
        git(&["remote", "add", "ssh", "git@github.com:ozkl/other.git"]);
        git(&[
            "remote",
            "add",
            "pinned",
            "https://github.com/work-alp/tool.git",
        ]);
        git(&[
            "config",
            "credential.https://github.com/work-alp/tool.git.username",
            "someone-else",
        ]);

        set_accounts(vec!["ozkl".to_owned(), "work-alp".to_owned()]);
        // Not a network command: nothing added.
        assert!(credential_config(&dir, &["status"]).is_empty());
        // Owner matches a login, so no network probe happens. The SSH remote and the remote
        // with an explicitly configured account are left alone.
        let cfg = credential_config(&dir, &["fetch", "--all"]);
        assert_eq!(
            cfg,
            [
                "-c",
                "credential.https://github.com/ozkl/gitr.git.username=ozkl"
            ]
        );
        assert_eq!(
            cached_account_for_url("https://github.com/ozkl/gitr.git").as_deref(),
            Some("ozkl")
        );
        // Clone: the URL comes from the command line.
        let cfg = credential_config(
            &dir,
            &["clone", "https://github.com/work-alp/new.git", "dest"],
        );
        assert!(cfg.contains(
            &"credential.https://github.com/work-alp/new.git.username=work-alp".to_owned()
        ));
        set_accounts(Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With two accounts stored for github.com, the per-URL `credential.<url>.username`
    /// option (what `credential_config` emits) makes git select that account's token.
    #[test]
    fn per_url_username_selects_the_account() {
        let file = std::env::temp_dir().join(format!("gitr-cred2-{}", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let helper = format!("credential.helper=store --file={}", file.display());
        let base = ["-c", "credential.helper=", "-c", helper.as_str()];
        for (user, token) in [("ozkl", "token-personal"), ("work-alp", "token-work")] {
            let input =
                format!("protocol=https\nhost=github.com\nusername={user}\npassword={token}\n\n");
            git_credential_with(&base, "approve", &input).unwrap();
        }
        let token_for = |repo_url: &str, login: &str| {
            let option = format!("credential.{repo_url}.username={login}");
            let mut config = base.to_vec();
            config.extend(["-c", option.as_str()]);
            let out = git_credential_with(&config, "fill", &format!("url={repo_url}\n\n")).unwrap();
            out.lines()
                .find_map(|l| l.strip_prefix("password="))
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            token_for("https://github.com/acme/tool.git", "work-alp"),
            "token-work"
        );
        assert_eq!(
            token_for("https://github.com/ozkl/gitr.git", "ozkl"),
            "token-personal"
        );

        // Options for several repositories at once (as for `fetch --all`) do not interfere:
        // each URL gets its own account.
        let mut config = base.to_vec();
        config.extend([
            "-c",
            "credential.https://github.com/acme/tool.git.username=work-alp",
            "-c",
            "credential.https://github.com/ozkl/gitr.git.username=ozkl",
        ]);
        for (url, token) in [
            ("https://github.com/ozkl/gitr.git", "token-personal"),
            ("https://github.com/acme/tool.git", "token-work"),
        ] {
            let out = git_credential_with(&config, "fill", &format!("url={url}\n\n")).unwrap();
            assert!(
                out.lines().any(|l| l == format!("password={token}")),
                "{url}: {out}"
            );
        }
        let _ = std::fs::remove_file(&file);
    }
}
