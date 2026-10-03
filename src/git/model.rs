//! Data types describing repository state and the parsers that produce them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::cmd;

const FS: char = '\x1f';

#[derive(Debug, Clone)]
pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub subject: String,
}

impl Commit {
    pub fn short_id(&self) -> &str {
        short(&self.id)
    }
}

pub fn short(id: &str) -> &str {
    &id[..id.len().min(7)]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefKind,
    /// The local branch currently checked out.
    pub is_head: bool,
}

#[derive(Debug, Clone)]
pub struct Branch {
    /// Short name, e.g. `feature/x` or `origin/main`.
    pub name: String,
    pub target: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub upstream_gone: bool,
    pub is_head: bool,
}

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct Remote {
    pub name: String,
    pub url: String,
    /// Branch names without the `<remote>/` prefix.
    pub branches: Vec<Branch>,
}

#[derive(Debug, Clone)]
pub struct Stash {
    pub refname: String,
    pub id: String,
    pub message: String,
    pub time: i64,
}

#[derive(Debug, Clone)]
pub struct Submodule {
    pub path: String,
    pub status: char,
}

#[derive(Debug, Clone, Default)]
pub struct Refs {
    pub head_branch: Option<String>,
    pub head_id: Option<String>,
    pub branches: Vec<Branch>,
    pub remotes: Vec<Remote>,
    pub tags: Vec<Tag>,
    pub stashes: Vec<Stash>,
    pub submodules: Vec<Submodule>,
    /// Commit id -> labels to show in the graph.
    pub labels: HashMap<String, Vec<RefLabel>>,
    /// In-progress operation (merge, rebase, cherry-pick, ...), if any.
    pub state: Option<RepoState>,
    /// Names of the two sides while an operation is in progress.
    pub sides: Option<super::conflict::SideLabels>,
    /// Effective commit identity ("Name <email>"), if configured.
    pub identity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

impl RepoState {
    pub fn label(self) -> &'static str {
        match self {
            RepoState::Merging => "Merge in progress",
            RepoState::Rebasing => "Rebase in progress",
            RepoState::CherryPicking => "Cherry-pick in progress",
            RepoState::Reverting => "Revert in progress",
        }
    }

    /// The git subcommand that owns `--continue` / `--abort`.
    pub fn command(self) -> &'static str {
        match self {
            RepoState::Merging => "merge",
            RepoState::Rebasing => "rebase",
            RepoState::CherryPicking => "cherry-pick",
            RepoState::Reverting => "revert",
        }
    }
}

impl Refs {
    pub fn local_branch(&self, name: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.name == name)
    }

    pub fn head_upstream(&self) -> Option<&Branch> {
        self.head_branch.as_deref().and_then(|b| self.local_branch(b))
    }

    pub fn remote_names(&self) -> Vec<String> {
        self.remotes.iter().map(|r| r.name.clone()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl ChangeKind {
    fn from_char(c: char) -> ChangeKind {
        match c {
            'A' => ChangeKind::Added,
            'D' => ChangeKind::Deleted,
            'R' => ChangeKind::Renamed,
            'C' => ChangeKind::Copied,
            'T' => ChangeKind::TypeChanged,
            'U' => ChangeKind::Conflicted,
            '?' => ChangeKind::Untracked,
            _ => ChangeKind::Modified,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            ChangeKind::Added => "A",
            ChangeKind::Modified => "M",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Copied => "C",
            ChangeKind::TypeChanged => "T",
            ChangeKind::Untracked => "+",
            ChangeKind::Conflicted => "!",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    /// Changed paths tracked with Git LFS.
    pub lfs: HashSet<String>,
    /// Conflicted paths and how they conflict.
    pub conflicts: HashMap<String, super::conflict::ConflictKind>,
}

impl Status {
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }

    pub fn total(&self) -> usize {
        let mut paths: Vec<&str> = self
            .staged
            .iter()
            .chain(self.unstaged.iter())
            .map(|f| f.path.as_str())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths.len()
    }
}

#[derive(Debug, Clone)]
pub struct CommitDetails {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer: String,
    pub committer_email: String,
    pub commit_time: i64,
    pub message: String,
    pub files: Vec<FileChange>,
    /// Paths of `files` tracked with Git LFS (per the commit's attributes).
    pub lfs: HashSet<String>,
}

// ---------------------------------------------------------------------------
// Queries

pub fn repo_root(path: &Path) -> Result<PathBuf, String> {
    let out = cmd::run(path, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(out.trim()))
}

pub fn load_log(repo: &Path, limit: usize) -> Result<Vec<Commit>, String> {
    // A fresh repository (or an orphan branch) has no HEAD commit; `git log` would fail on it.
    let has_head = cmd::run(repo, &["rev-parse", "-q", "--verify", "HEAD^{commit}"]).is_ok();
    if !has_head {
        let any_ref = cmd::run(repo, &["for-each-ref", "--count=1", "refs/heads", "refs/remotes", "refs/tags"])?;
        if any_ref.trim().is_empty() {
            return Ok(Vec::new());
        }
    }
    let limit = format!("--max-count={limit}");
    let mut args = vec!["log", "--branches", "--remotes", "--tags"];
    if has_head {
        args.push("HEAD");
    }
    args.extend(["--date-order", "-z", &limit, "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s", "--"]);
    let out = cmd::run(repo, &args)?;
    Ok(out
        .split('\0')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').split(FS);
            Some(Commit {
                id: f.next()?.to_owned(),
                parents: f.next()?.split_whitespace().map(str::to_owned).collect(),
                author: f.next()?.to_owned(),
                email: f.next()?.to_owned(),
                time: f.next()?.parse().unwrap_or(0),
                subject: f.next().unwrap_or_default().to_owned(),
            })
        })
        .collect())
}

fn parse_track(track: &str) -> (u32, u32, bool) {
    // "[ahead 1, behind 2]", "[gone]", ""
    let inner = track.trim().trim_start_matches('[').trim_end_matches(']');
    if inner == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in inner.split(',') {
        let mut words = part.split_whitespace();
        match (words.next(), words.next()) {
            (Some("ahead"), Some(n)) => ahead = n.parse().unwrap_or(0),
            (Some("behind"), Some(n)) => behind = n.parse().unwrap_or(0),
            _ => {}
        }
    }
    (ahead, behind, false)
}

pub fn load_refs(repo: &Path) -> Result<Refs, String> {
    let mut refs = Refs::default();

    let out = cmd::run(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname)%1f%(objectname)%1f%(*objectname)%1f%(upstream:short)%1f%(upstream:track)%1f%(HEAD)%1f%(objecttype)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
    )?;

    let remote_urls = remote_urls(repo);
    let mut remotes: Vec<Remote> = remote_urls
        .iter()
        .map(|(name, url)| Remote {
            name: name.clone(),
            url: url.clone(),
            branches: Vec::new(),
        })
        .collect();

    for line in out.lines() {
        let f: Vec<&str> = line.split(FS).collect();
        if f.len() < 7 {
            continue;
        }
        let refname = f[0];
        let target = if f[2].is_empty() { f[1] } else { f[2] }.to_owned();
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            let (ahead, behind, gone) = parse_track(f[4]);
            let is_head = f[5] == "*";
            refs.branches.push(Branch {
                name: name.to_owned(),
                target: target.clone(),
                upstream: (!f[3].is_empty()).then(|| f[3].to_owned()),
                ahead,
                behind,
                upstream_gone: gone,
                is_head,
            });
            refs.labels.entry(target).or_default().push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::LocalBranch,
                is_head,
            });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            if name.ends_with("/HEAD") {
                continue;
            }
            let Some((remote, branch)) = split_remote(name, &remote_urls) else {
                continue;
            };
            let entry = match remotes.iter_mut().find(|r| r.name == remote) {
                Some(r) => r,
                None => {
                    remotes.push(Remote {
                        name: remote.to_owned(),
                        url: String::new(),
                        branches: Vec::new(),
                    });
                    remotes.last_mut().unwrap()
                }
            };
            entry.branches.push(Branch {
                name: branch.to_owned(),
                target: target.clone(),
                upstream: None,
                ahead: 0,
                behind: 0,
                upstream_gone: false,
                is_head: false,
            });
            refs.labels.entry(target).or_default().push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::RemoteBranch,
                is_head: false,
            });
        } else if let Some(name) = refname.strip_prefix("refs/tags/") {
            refs.tags.push(Tag {
                name: name.to_owned(),
                target: target.clone(),
            });
            refs.labels.entry(target).or_default().push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::Tag,
                is_head: false,
            });
        }
    }
    refs.remotes = remotes;
    refs.tags.sort_by(|a, b| natural_cmp(&b.name, &a.name));

    refs.head_id = cmd::run(repo, &["rev-parse", "-q", "--verify", "HEAD"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    refs.head_branch = cmd::run(repo, &["symbolic-ref", "-q", "--short", "HEAD"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if refs.head_branch.is_none() {
        if let Some(id) = &refs.head_id {
            refs.labels.entry(id.clone()).or_default().insert(
                0,
                RefLabel {
                    name: "HEAD".into(),
                    kind: RefKind::Head,
                    is_head: true,
                },
            );
        }
    }

    refs.stashes = load_stashes(repo);
    refs.submodules = load_submodules(repo);
    refs.state = repo_state(repo);
    refs.sides = refs.state.map(|s| super::conflict::side_labels(repo, s, refs.head_branch.as_deref()));
    refs.identity = cmd::run(repo, &["var", "GIT_AUTHOR_IDENT"]).ok().map(|s| strip_ident_date(s.trim()));
    Ok(refs)
}

fn split_remote<'a>(name: &'a str, remotes: &[(String, String)]) -> Option<(&'a str, &'a str)> {
    // Prefer the longest matching remote name (remote names may contain '/').
    remotes
        .iter()
        .filter(|(r, _)| name.len() > r.len() && name.starts_with(r.as_str()) && name.as_bytes()[r.len()] == b'/')
        .max_by_key(|(r, _)| r.len())
        .map(|(r, _)| (&name[..r.len()], &name[r.len() + 1..]))
        .or_else(|| name.split_once('/'))
}

fn remote_urls(repo: &Path) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    if let Ok(out) = cmd::run(repo, &["remote", "-v"]) {
        for line in out.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(name), Some(url)) = (parts.next(), parts.next()) {
                if !result.iter().any(|(n, _)| n == name) {
                    result.push((name.to_owned(), url.to_owned()));
                }
            }
        }
    }
    // Remotes without URLs still show up in `git remote`.
    if let Ok(out) = cmd::run(repo, &["remote"]) {
        for name in out.lines().map(str::trim).filter(|n| !n.is_empty()) {
            if !result.iter().any(|(n, _)| n == name) {
                result.push((name.to_owned(), String::new()));
            }
        }
    }
    result
}

fn load_stashes(repo: &Path) -> Vec<Stash> {
    let Ok(out) = cmd::run(repo, &["stash", "list", "-z", "--format=%gd%x1f%H%x1f%gs%x1f%ct"]) else {
        return Vec::new();
    };
    out.split('\0')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').split(FS);
            Some(Stash {
                refname: f.next()?.to_owned(),
                id: f.next()?.to_owned(),
                message: f.next()?.to_owned(),
                time: f.next().and_then(|t| t.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

fn load_submodules(repo: &Path) -> Vec<Submodule> {
    if !repo.join(".gitmodules").exists() {
        return Vec::new();
    }
    let Ok(out) = cmd::run(repo, &["submodule", "status"]) else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|line| {
            let status = line.chars().next()?;
            let mut parts = line[1..].split_whitespace();
            Some(Submodule {
                path: parts.nth(1)?.to_owned(),
                status,
            })
        })
        .collect()
}

pub fn git_dir(repo: &Path) -> PathBuf {
    cmd::run(repo, &["rev-parse", "--absolute-git-dir"])
        .map(|s| PathBuf::from(s.trim()))
        .unwrap_or_else(|_| repo.join(".git"))
}

fn repo_state(repo: &Path) -> Option<RepoState> {
    let dir = git_dir(repo);
    if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
        Some(RepoState::Rebasing)
    } else if dir.join("MERGE_HEAD").exists() {
        Some(RepoState::Merging)
    } else if dir.join("CHERRY_PICK_HEAD").exists() {
        Some(RepoState::CherryPicking)
    } else if dir.join("REVERT_HEAD").exists() {
        Some(RepoState::Reverting)
    } else {
        None
    }
}

pub fn load_status(repo: &Path) -> Result<Status, String> {
    let out = cmd::run(
        repo,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all", "--ignore-submodules=dirty"],
    )?;
    let mut status = Status::default();
    let mut fields = out.split('\0');
    while let Some(entry) = fields.next() {
        if entry.is_empty() {
            continue;
        }
        let kind = entry.as_bytes()[0];
        match kind {
            b'1' | b'2' => {
                let parts: Vec<&str> = entry.splitn(if kind == b'1' { 9 } else { 10 }, ' ').collect();
                let xy: Vec<char> = parts[1].chars().collect();
                let path = parts.last().copied().unwrap_or_default().to_owned();
                let old_path = if kind == b'2' {
                    fields.next().map(str::to_owned)
                } else {
                    None
                };
                if xy[0] != '.' {
                    status.staged.push(FileChange {
                        path: path.clone(),
                        old_path: old_path.clone(),
                        kind: ChangeKind::from_char(xy[0]),
                    });
                }
                if xy[1] != '.' {
                    status.unstaged.push(FileChange {
                        path,
                        old_path: None,
                        kind: ChangeKind::from_char(xy[1]),
                    });
                }
            }
            b'u' => {
                let parts: Vec<&str> = entry.splitn(11, ' ').collect();
                let path = parts.last().copied().unwrap_or_default().to_owned();
                if let Some(kind) = parts.get(1).and_then(|xy| super::conflict::ConflictKind::from_xy(xy)) {
                    status.conflicts.insert(path.clone(), kind);
                }
                status.unstaged.push(FileChange { path, old_path: None, kind: ChangeKind::Conflicted });
            }
            b'?' => status.unstaged.push(FileChange {
                path: entry[2..].to_owned(),
                old_path: None,
                kind: ChangeKind::Untracked,
            }),
            _ => {}
        }
    }
    let sort = |v: &mut Vec<FileChange>| v.sort_by(|a, b| a.path.to_lowercase().cmp(&b.path.to_lowercase()));
    sort(&mut status.staged);
    sort(&mut status.unstaged);
    let mut paths: Vec<String> = status.staged.iter().chain(&status.unstaged).map(|f| f.path.clone()).collect();
    paths.sort_unstable();
    paths.dedup();
    status.lfs = super::lfs::lfs_paths(repo, None, &paths);
    Ok(status)
}

fn parse_name_status(out: &str) -> Vec<FileChange> {
    let mut files = Vec::new();
    let mut fields = out.split('\0').filter(|s| !s.is_empty());
    while let Some(code) = fields.next() {
        let letter = code.chars().next().unwrap_or('M');
        let kind = ChangeKind::from_char(letter);
        if matches!(kind, ChangeKind::Renamed | ChangeKind::Copied) {
            let old = fields.next().unwrap_or_default().to_owned();
            let new = fields.next().unwrap_or_default().to_owned();
            files.push(FileChange {
                path: new,
                old_path: Some(old),
                kind,
            });
        } else if let Some(path) = fields.next() {
            files.push(FileChange {
                path: path.to_owned(),
                old_path: None,
                kind,
            });
        }
    }
    files
}

pub fn load_commit_details(repo: &Path, id: &str) -> Result<CommitDetails, String> {
    let out = cmd::run(
        repo,
        &[
            "show",
            "-s",
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%B",
            id,
            "--",
        ],
    )?;
    let f: Vec<&str> = out.splitn(9, FS).collect();
    if f.len() < 9 {
        return Err(format!("Unexpected output for commit {id}"));
    }
    let parents: Vec<String> = f[1].split_whitespace().map(str::to_owned).collect();
    let files = commit_files(repo, id, parents.first().map(String::as_str))?;
    Ok(CommitDetails {
        id: f[0].to_owned(),
        parents,
        author: f[2].to_owned(),
        author_email: f[3].to_owned(),
        author_time: f[4].parse().unwrap_or(0),
        committer: f[5].to_owned(),
        committer_email: f[6].to_owned(),
        commit_time: f[7].parse().unwrap_or(0),
        message: f[8].trim_end().to_owned(),
        lfs: super::lfs::lfs_paths(repo, Some(id), &files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()),
        files,
    })
}

pub fn commit_files(repo: &Path, id: &str, parent: Option<&str>) -> Result<Vec<FileChange>, String> {
    let out = match parent {
        Some(p) => cmd::run(repo, &["diff-tree", "-r", "-z", "-M", "--name-status", p, id])?,
        None => cmd::run(repo, &["diff-tree", "-r", "-z", "-M", "--name-status", "--root", "--no-commit-id", id])?,
    };
    Ok(parse_name_status(&out))
}

/// A commit that touched a file, and how the file changed in it.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub commit: Commit,
    pub file: FileChange,
}

/// Commits reachable from `rev` that changed `path`, newest first, following renames.
pub fn file_history(repo: &Path, rev: &str, path: &str, limit: usize) -> Result<Vec<HistoryEntry>, String> {
    let limit = format!("--max-count={limit}");
    let out = cmd::run(
        repo,
        &[
            "log",
            "--follow",
            "-M",
            "-z",
            "--name-status",
            &limit,
            "--format=%x1e%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s",
            rev,
            "--",
            path,
        ],
    )?;
    Ok(parse_file_history(&out, path))
}

fn parse_file_history(out: &str, path: &str) -> Vec<HistoryEntry> {
    // The file's path as of the entry being parsed; changes at renames (walking back in time).
    let mut current = path.to_owned();
    let mut entries = Vec::new();
    for record in out.split('\x1e').filter(|r| !r.trim().is_empty()) {
        let (header, changes) = record.split_once('\0').unwrap_or((record, ""));
        let mut f = header.split(FS);
        let (Some(id), Some(parents), Some(author), Some(email), Some(time)) = (f.next(), f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let commit = Commit {
            id: id.to_owned(),
            parents: parents.split_whitespace().map(str::to_owned).collect(),
            author: author.to_owned(),
            email: email.to_owned(),
            time: time.parse().unwrap_or(0),
            subject: f.next().unwrap_or_default().to_owned(),
        };
        let changes = changes.trim_start_matches('\n');
        // Merge commits list no changes; the file keeps its current path there.
        let file = parse_name_status(changes)
            .into_iter()
            .next()
            .unwrap_or(FileChange { path: current.clone(), old_path: None, kind: ChangeKind::Modified });
        current = file.old_path.clone().unwrap_or_else(|| file.path.clone());
        entries.push(HistoryEntry { commit, file });
    }
    entries
}

/// All file paths in a commit, and which of them are tracked with Git LFS.
pub fn list_tree(repo: &Path, id: &str) -> Result<(Vec<String>, HashSet<String>), String> {
    let out = cmd::run(repo, &["ls-tree", "-r", "-z", "--name-only", id])?;
    let paths: Vec<String> = out.split('\0').filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let lfs = super::lfs::lfs_paths(repo, Some(id), &paths);
    Ok((paths, lfs))
}

pub fn file_at(repo: &Path, id: &str, path: &str) -> Result<Vec<u8>, String> {
    cmd::run_bytes(repo, &["show", &format!("{id}:{path}")])
}

/// "Name <email> 1700000000 +0300" -> "Name <email>"
fn strip_ident_date(ident: &str) -> String {
    match ident.rfind('>') {
        Some(i) => ident[..=i].to_owned(),
        None => ident.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Repository-local settings

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullMode {
    /// Not set for this repository: the global setting applies.
    Inherit,
    Merge,
    Rebase,
    FastForwardOnly,
}

impl PullMode {
    pub fn label(self) -> &'static str {
        match self {
            PullMode::Inherit => "Use global setting",
            PullMode::Merge => "Merge",
            PullMode::Rebase => "Rebase",
            PullMode::FastForwardOnly => "Fast-forward only",
        }
    }

    fn from_config(rebase: Option<&str>, ff: Option<&str>) -> PullMode {
        match (rebase, ff) {
            (Some(r), _) if r != "false" => PullMode::Rebase,
            (_, Some("only")) => PullMode::FastForwardOnly,
            (Some(_), _) => PullMode::Merge,
            (None, _) => PullMode::Inherit,
        }
    }
}

/// Per-repository git settings edited in the Repository Settings dialog.
/// Empty strings / `Inherit` / `None` mean "not set locally".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoConfig {
    pub user_name: String,
    pub user_email: String,
    pub pull: PullMode,
    pub prune: Option<bool>,
}

/// Values that apply when the repository does not override them (global / system config).
#[derive(Debug, Clone, Default)]
pub struct InheritedConfig {
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    pub pull: String,
    pub prune: bool,
}

fn config_at(repo: &Path, scope: &str, key: &str) -> Option<String> {
    cmd::run(repo, &["config", scope, "--get", key])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn inherited(repo: &Path, key: &str) -> Option<String> {
    config_at(repo, "--global", key).or_else(|| config_at(repo, "--system", key))
}

pub fn load_repo_config(repo: &Path) -> (RepoConfig, InheritedConfig) {
    let local = |k: &str| config_at(repo, "--local", k);
    let config = RepoConfig {
        user_name: local("user.name").unwrap_or_default(),
        user_email: local("user.email").unwrap_or_default(),
        pull: PullMode::from_config(local("pull.rebase").as_deref(), local("pull.ff").as_deref()),
        prune: local("fetch.prune").map(|v| v == "true"),
    };
    let pull = match PullMode::from_config(inherited(repo, "pull.rebase").as_deref(), inherited(repo, "pull.ff").as_deref()) {
        PullMode::Inherit => PullMode::Merge,
        m => m,
    };
    let inherited = InheritedConfig {
        user_name: inherited(repo, "user.name"),
        user_email: inherited(repo, "user.email"),
        pull: pull.label().to_owned(),
        prune: inherited(repo, "fetch.prune").is_some_and(|v| v == "true"),
    };
    (config, inherited)
}

/// Writes the settings that changed between `old` and `new` to the repository's config.
pub fn save_repo_config(repo: &Path, old: &RepoConfig, new: &RepoConfig) -> Result<String, String> {
    let set = |k: &str, v: &str| cmd::run(repo, &["config", "--local", k, v]).map(|_| ());
    // `--unset` fails with exit code 5 when the key is absent; that is fine here.
    let unset = |k: &str| {
        let _ = cmd::run(repo, &["config", "--local", "--unset-all", k]);
        Ok::<(), String>(())
    };
    let mut changed = Vec::new();
    for (key, o, n) in [("user.name", &old.user_name, &new.user_name), ("user.email", &old.user_email, &new.user_email)] {
        let n = n.trim();
        if o.trim() != n {
            if n.is_empty() { unset(key)? } else { set(key, n)? }
            changed.push(key);
        }
    }
    if old.pull != new.pull {
        match new.pull {
            PullMode::Inherit => {
                unset("pull.rebase")?;
                unset("pull.ff")?;
            }
            PullMode::Merge => {
                set("pull.rebase", "false")?;
                unset("pull.ff")?;
            }
            PullMode::Rebase => {
                set("pull.rebase", "true")?;
                unset("pull.ff")?;
            }
            PullMode::FastForwardOnly => {
                unset("pull.rebase")?;
                set("pull.ff", "only")?;
            }
        }
        changed.push("pull");
    }
    if old.prune != new.prune {
        match new.prune {
            None => unset("fetch.prune")?,
            Some(v) => set("fetch.prune", if v { "true" } else { "false" })?,
        }
        changed.push("fetch.prune");
    }
    Ok(if changed.is_empty() { "No changes".into() } else { format!("Updated {}", changed.join(", ")) })
}

pub fn config_value(repo: &Path, key: &str) -> Option<String> {
    cmd::run(repo, &["config", "--get", key])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Last commit message of HEAD (for amend).
pub fn head_message(repo: &Path) -> Option<String> {
    cmd::run(repo, &["log", "-1", "--format=%B"]).ok().map(|s| s.trim_end().to_owned())
}

/// Compares strings treating digit runs as numbers (v1.10 > v1.9).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek(), bi.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na: String = std::iter::from_fn(|| ai.next_if(char::is_ascii_digit)).collect();
                let nb: String = std::iter::from_fn(|| bi.next_if(char::is_ascii_digit)).collect();
                let ord = na.len().cmp(&nb.len()).then(na.cmp(&nb));
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            (Some(_), Some(_)) => {
                let (x, y) = (ai.next().unwrap(), bi.next().unwrap());
                if x != y {
                    return x.cmp(&y);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracking_info() {
        assert_eq!(parse_track("[ahead 2, behind 3]"), (2, 3, false));
        assert_eq!(parse_track("[behind 1]"), (0, 1, false));
        assert_eq!(parse_track("[gone]"), (0, 0, true));
        assert_eq!(parse_track(""), (0, 0, false));
    }

    #[test]
    fn parses_name_status_with_renames() {
        let files = parse_name_status("M\0a.rs\0R100\0old.rs\0new.rs\0A\0b.rs\0");
        assert_eq!(files.len(), 3);
        assert_eq!(files[1].kind, ChangeKind::Renamed);
        assert_eq!(files[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(files[1].path, "new.rs");
    }

    #[test]
    fn natural_ordering() {
        assert_eq!(natural_cmp("v1.10", "v1.9"), std::cmp::Ordering::Greater);
        assert_eq!(natural_cmp("a", "b"), std::cmp::Ordering::Less);
    }

    #[test]
    fn empty_repository_has_no_commits_and_no_error() {
        let dir = std::env::temp_dir().join(format!("gitr-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        cmd::run(&dir, &["init", "-q"]).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        assert!(load_log(&dir, 100).unwrap().is_empty());
        let refs = load_refs(&dir).unwrap();
        assert!(refs.head_id.is_none());
        assert_eq!(load_status(&dir).unwrap().unstaged.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_at_returns_exact_bytes_of_that_commit() {
        let dir = std::env::temp_dir().join(format!("gitr-fileat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let git = |args: &[&str]| cmd::run(&dir, args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        let v1: Vec<u8> = vec![0, 159, 146, 150, b'\r', b'\n', 255];
        std::fs::write(dir.join("sub/data.bin"), &v1).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "v1"]);
        let first = git(&["rev-parse", "HEAD"]).trim().to_owned();
        std::fs::write(dir.join("sub/data.bin"), b"v2").unwrap();
        git(&["commit", "-qam", "v2"]);
        assert_eq!(file_at(&dir, &first, "sub/data.bin").unwrap(), v1);
        assert_eq!(file_at(&dir, "HEAD", "sub/data.bin").unwrap(), b"v2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_config_round_trip() {
        let dir = std::env::temp_dir().join(format!("gitr-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        cmd::run(&dir, &["init", "-q"]).unwrap();
        let (initial, _) = load_repo_config(&dir);
        assert_eq!(initial.pull, PullMode::Inherit);
        assert!(initial.user_name.is_empty());

        let new = RepoConfig {
            user_name: "Ada Lovelace".into(),
            user_email: "ada@example.com".into(),
            pull: PullMode::FastForwardOnly,
            prune: Some(true),
        };
        save_repo_config(&dir, &initial, &new).unwrap();
        assert_eq!(load_repo_config(&dir).0, new);
        assert_eq!(config_at(&dir, "--local", "pull.ff").as_deref(), Some("only"));

        // Clearing returns to inherited values.
        save_repo_config(&dir, &new, &initial).unwrap();
        assert_eq!(load_repo_config(&dir).0, initial);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strips_ident_timestamp() {
        assert_eq!(strip_ident_date("Ada L <ada@x.com> 1700000000 +0300"), "Ada L <ada@x.com>");
    }

    #[test]
    fn file_history_follows_renames() {
        let dir = std::env::temp_dir().join(format!("gitr-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| cmd::run(&dir, args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        let body: String = (0..40).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join("old.txt"), &body).unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "create"]);
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "unrelated"]);
        git(&["mv", "old.txt", "new.txt"]);
        git(&["commit", "-qm", "rename"]);
        std::fs::write(dir.join("new.txt"), format!("{body}more\n")).unwrap();
        git(&["commit", "-qam", "edit"]);

        let h = file_history(&dir, "HEAD", "new.txt", 100).unwrap();
        let subjects: Vec<&str> = h.iter().map(|e| e.commit.subject.as_str()).collect();
        assert_eq!(subjects, ["edit", "rename", "create"]);
        assert_eq!(h[0].file.kind, ChangeKind::Modified);
        assert_eq!(h[1].file.kind, ChangeKind::Renamed);
        assert_eq!(h[1].file.old_path.as_deref(), Some("old.txt"));
        assert_eq!((h[2].file.kind, h[2].file.path.as_str()), (ChangeKind::Added, "old.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
