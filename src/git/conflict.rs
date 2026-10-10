//! Merge conflicts: what each side did, who the sides are, conflict-marker parsing and
//! writing a resolution back.

use std::path::Path;

use super::cmd;
use super::model::{RepoState, git_dir};

/// How a path conflicts, from the `XY` code of `git status --porcelain=v2` "u" entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    BothModified,
    BothAdded,
    BothDeleted,
    AddedByUs,
    AddedByThem,
    DeletedByUs,
    DeletedByThem,
}

/// What one side did to the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideChange {
    Modified,
    Added,
    Deleted,
    /// The file does not exist on that side.
    Absent,
}

impl SideChange {
    pub fn label(self) -> &'static str {
        match self {
            SideChange::Modified => "modified",
            SideChange::Added => "added",
            SideChange::Deleted => "deleted",
            SideChange::Absent => "not present",
        }
    }

    pub fn exists(self) -> bool {
        matches!(self, SideChange::Modified | SideChange::Added)
    }
}

impl ConflictKind {
    pub fn from_xy(xy: &str) -> Option<ConflictKind> {
        Some(match xy {
            "UU" => ConflictKind::BothModified,
            "AA" => ConflictKind::BothAdded,
            "DD" => ConflictKind::BothDeleted,
            "AU" => ConflictKind::AddedByUs,
            "UA" => ConflictKind::AddedByThem,
            "DU" => ConflictKind::DeletedByUs,
            "UD" => ConflictKind::DeletedByThem,
            _ => return None,
        })
    }

    /// (ours, theirs)
    pub fn sides(self) -> (SideChange, SideChange) {
        use SideChange::*;
        match self {
            ConflictKind::BothModified => (Modified, Modified),
            ConflictKind::BothAdded => (Added, Added),
            ConflictKind::BothDeleted => (Deleted, Deleted),
            ConflictKind::AddedByUs => (Added, Absent),
            ConflictKind::AddedByThem => (Absent, Added),
            ConflictKind::DeletedByUs => (Deleted, Modified),
            ConflictKind::DeletedByThem => (Modified, Deleted),
        }
    }

    /// Whether the working file contains conflict markers that can be merged line by line.
    pub fn has_markers(self) -> bool {
        matches!(self, ConflictKind::BothModified | ConflictKind::BothAdded)
    }
}

/// Names for the two sides of an in-progress operation: (ours, theirs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideLabels {
    pub ours: String,
    pub theirs: String,
}

fn read_trimmed(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// `abc1234 subject` for a revision.
fn describe_commit(repo: &Path, rev: &str) -> Option<String> {
    cmd::run(repo, &["log", "-1", "--format=%h %s", rev, "--"])
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// A branch name pointing at `rev`, if there is one.
fn branch_at(repo: &Path, rev: &str) -> Option<String> {
    let out = cmd::run(
        repo,
        &[
            "name-rev",
            "--name-only",
            "--no-undefined",
            "--refs=refs/heads/*",
            "--refs=refs/remotes/*",
            rev,
        ],
    )
    .ok()?;
    let name = out.trim().trim_start_matches("remotes/");
    (!name.is_empty() && !name.contains('~') && !name.contains('^')).then(|| name.to_owned())
}

/// The branch named in `MERGE_MSG` ("Merge branch 'x'", "Merge remote-tracking branch 'origin/x'").
fn merge_source(msg: &str) -> Option<String> {
    let first = msg.lines().next()?;
    let start = first.find('\'')? + 1;
    let end = start + first[start..].find('\'')?;
    Some(first[start..end].to_owned())
}

pub fn side_labels(repo: &Path, state: RepoState, head_branch: Option<&str>) -> SideLabels {
    let dir = git_dir(repo);
    let head = head_branch
        .map(str::to_owned)
        .or_else(|| describe_commit(repo, "HEAD"))
        .unwrap_or_else(|| "HEAD".into());
    match state {
        RepoState::Merging => {
            let theirs = read_trimmed(&dir.join("MERGE_MSG"))
                .and_then(|m| merge_source(&m))
                .or_else(|| {
                    read_trimmed(&dir.join("MERGE_HEAD")).and_then(|sha| {
                        branch_at(repo, &sha).or_else(|| describe_commit(repo, &sha))
                    })
                })
                .unwrap_or_else(|| "MERGE_HEAD".into());
            SideLabels { ours: head, theirs }
        }
        RepoState::Rebasing => {
            // During a rebase "ours" is the branch being rebased onto and "theirs" is the
            // commit being replayed.
            let onto = ["rebase-merge/onto", "rebase-apply/onto"]
                .iter()
                .find_map(|f| read_trimmed(&dir.join(f)));
            let ours = onto
                .map(|sha| {
                    branch_at(repo, &sha)
                        .unwrap_or_else(|| describe_commit(repo, &sha).unwrap_or(sha))
                })
                .unwrap_or_else(|| "upstream".into());
            let branch = ["rebase-merge/head-name", "rebase-apply/head-name"]
                .iter()
                .find_map(|f| read_trimmed(&dir.join(f)))
                .map(|h| h.trim_start_matches("refs/heads/").to_owned());
            let commit = describe_commit(repo, "REBASE_HEAD")
                .unwrap_or_else(|| "commit being applied".into());
            let theirs = match branch {
                Some(b) => format!("{commit} ({b})"),
                None => commit,
            };
            SideLabels { ours, theirs }
        }
        RepoState::CherryPicking => SideLabels {
            ours: head,
            theirs: describe_commit(repo, "CHERRY_PICK_HEAD")
                .unwrap_or_else(|| "picked commit".into()),
        },
        RepoState::Reverting => SideLabels {
            ours: head,
            theirs: describe_commit(repo, "REVERT_HEAD")
                .map(|c| format!("revert of {c}"))
                .unwrap_or_else(|| "revert".into()),
        },
    }
}

/// The message git prepared for the merge/cherry-pick/revert commit, without comment lines.
pub fn prepared_message(repo: &Path) -> Option<String> {
    let text = std::fs::read_to_string(git_dir(repo).join("MERGE_MSG")).ok()?;
    let msg: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    let msg = msg.join("\n").trim().to_owned();
    (!msg.is_empty()).then_some(msg)
}

/// Git's draft for a pending squash merge (`SQUASH_MSG`), if there is one.
pub fn squash_draft(repo: &Path) -> Option<String> {
    std::fs::read_to_string(git_dir(repo).join("SQUASH_MSG"))
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// A tidy commit message for a squash merge, from git's draft (a raw `git log` dump).
///
/// One squashed commit: its own subject. Several: a subject naming `source`, then one
/// bullet per commit, oldest first.
pub fn squash_message(draft: &str, source: Option<&str>) -> String {
    // The draft lists commits newest first; a commit's subject is the first indented
    // line after its `commit <id>` header.
    let mut subjects = Vec::new();
    let mut want_subject = false;
    for line in draft.lines() {
        if line.starts_with("commit ") {
            want_subject = true;
        } else if want_subject && line.starts_with("    ") && !line.trim().is_empty() {
            subjects.push(line.trim().to_owned());
            want_subject = false;
        }
    }
    subjects.reverse();
    let title = match source {
        // A bare commit id reads better shortened and without "branch".
        Some(s) if s.len() >= 12 && s.chars().all(|c| c.is_ascii_hexdigit()) => {
            format!("Squash merge commit '{}'", &s[..7])
        }
        Some(s) => format!("Squash merge branch '{s}'"),
        None => "Squash merge".to_owned(),
    };
    match subjects.as_slice() {
        [] => title,
        [only] => only.clone(),
        many => {
            let bullets: Vec<String> = many.iter().map(|s| format!("* {s}")).collect();
            format!("{title}\n\n{}", bullets.join("\n"))
        }
    }
}

/// Git commands that resolve `path` by taking one side entirely.
pub fn take_side_commands(path: &str, kind: ConflictKind, ours: bool) -> Vec<Vec<String>> {
    let (o, t) = kind.sides();
    let chosen = if ours { o } else { t };
    let p = path.to_owned();
    if chosen.exists() {
        let flag = if ours { "--ours" } else { "--theirs" };
        vec![
            vec!["checkout".into(), flag.into(), "--".into(), p.clone()],
            vec!["add".into(), "--".into(), p],
        ]
    } else {
        // The chosen side has no file: resolve by removing it.
        vec![vec![
            "rm".into(),
            "--quiet".into(),
            "--ignore-unmatch".into(),
            "--".into(),
            p,
        ]]
    }
}

// ---------------------------------------------------------------------------
// Conflict markers

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    OursThenTheirs,
    TheirsThenOurs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub ours: String,
    /// Common ancestor version (only with `merge.conflictStyle = diff3/zdiff3`).
    pub base: Option<String>,
    pub theirs: String,
    pub choice: Option<Choice>,
}

impl Hunk {
    pub fn resolved_text(&self) -> Option<String> {
        Some(match self.choice? {
            Choice::Ours => self.ours.clone(),
            Choice::Theirs => self.theirs.clone(),
            Choice::OursThenTheirs => join_blocks(&self.ours, &self.theirs),
            Choice::TheirsThenOurs => join_blocks(&self.theirs, &self.ours),
        })
    }
}

/// Concatenates two blocks of lines, making sure the first ends with a newline.
fn join_blocks(a: &str, b: &str) -> String {
    let mut s = a.to_owned();
    if !s.is_empty() && !s.ends_with('\n') && !b.is_empty() {
        s.push('\n');
    }
    s.push_str(b);
    s
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Conflict(Hunk),
}

fn is_marker(line: &str, c: char) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    let n = line.chars().take_while(|&x| x == c).count();
    // `=======` stands alone; the others may be followed by a label.
    n == 7 && (line.len() == 7 || (c != '=' && line.as_bytes()[7] == b' '))
}

/// Splits a file containing conflict markers into text and conflict hunks.
/// Returns `None` when there are no (well-formed) markers.
pub fn parse(text: &str) -> Option<Vec<Segment>> {
    enum State {
        Text,
        Ours,
        Base,
        Theirs,
    }
    let mut segments = Vec::new();
    let mut text_buf = String::new();
    let mut hunk = Hunk {
        ours: String::new(),
        base: None,
        theirs: String::new(),
        choice: None,
    };
    let mut state = State::Text;
    let mut found = false;
    for line in text.split_inclusive('\n') {
        match state {
            State::Text if is_marker(line, '<') => {
                if !text_buf.is_empty() {
                    segments.push(Segment::Text(std::mem::take(&mut text_buf)));
                }
                state = State::Ours;
            }
            State::Text => text_buf.push_str(line),
            State::Ours if is_marker(line, '|') => {
                hunk.base = Some(String::new());
                state = State::Base;
            }
            State::Ours | State::Base if is_marker(line, '=') => state = State::Theirs,
            State::Ours => hunk.ours.push_str(line),
            State::Base => hunk.base.get_or_insert_with(String::new).push_str(line),
            State::Theirs if is_marker(line, '>') => {
                segments.push(Segment::Conflict(std::mem::replace(
                    &mut hunk,
                    Hunk {
                        ours: String::new(),
                        base: None,
                        theirs: String::new(),
                        choice: None,
                    },
                )));
                found = true;
                state = State::Text;
            }
            State::Theirs => hunk.theirs.push_str(line),
        }
    }
    if !matches!(state, State::Text) || !found {
        return None;
    }
    if !text_buf.is_empty() {
        segments.push(Segment::Text(text_buf));
    }
    Some(segments)
}

/// The resolved file, or `None` while some hunk has no choice.
pub fn render(segments: &[Segment]) -> Option<String> {
    let mut out = String::new();
    for s in segments {
        match s {
            Segment::Text(t) => out.push_str(t),
            Segment::Conflict(h) => out.push_str(&h.resolved_text()?),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "a\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> feature\nb\n<<<<<<< HEAD\nx\n||||||| base\nbase\n=======\ny\n>>>>>>> feature\nc\n";

    #[test]
    fn parses_and_renders_hunks() {
        let mut segs = parse(FILE).unwrap();
        let hunks: Vec<&Hunk> = segs
            .iter()
            .filter_map(|s| match s {
                Segment::Conflict(h) => Some(h),
                _ => None,
            })
            .collect();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].ours, "ours 1\nours 2\n");
        assert_eq!(hunks[0].theirs, "theirs\n");
        assert_eq!(hunks[1].base.as_deref(), Some("base\n"));
        assert_eq!(render(&segs), None);

        let mut n = 0;
        for s in &mut segs {
            if let Segment::Conflict(h) = s {
                h.choice = Some(if n == 0 {
                    Choice::TheirsThenOurs
                } else {
                    Choice::Ours
                });
                n += 1;
            }
        }
        assert_eq!(
            render(&segs).unwrap(),
            "a\ntheirs\nours 1\nours 2\nb\nx\nc\n"
        );
    }

    #[test]
    fn keeps_crlf_and_rejects_files_without_markers() {
        let crlf = "a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> other\r\nb\r\n";
        let mut segs = parse(crlf).unwrap();
        if let Segment::Conflict(h) = &mut segs[1] {
            h.choice = Some(Choice::Theirs);
        }
        assert_eq!(render(&segs).unwrap(), "a\r\ny\r\nb\r\n");
        assert!(parse("no conflicts\n").is_none());
        assert!(parse("<<<<<<< HEAD\nunterminated\n").is_none());
        // "=======" inside normal text (e.g. a markdown underline) is not a marker.
        assert!(parse("Title\n=======\n").is_none());
    }

    #[test]
    fn side_commands_remove_deleted_sides() {
        let cmds = take_side_commands("f.txt", ConflictKind::DeletedByThem, false);
        assert_eq!(cmds[0][0], "rm");
        let cmds = take_side_commands("f.txt", ConflictKind::DeletedByThem, true);
        assert_eq!(cmds[0][..2], ["checkout".to_owned(), "--ours".to_owned()]);
    }

    #[test]
    fn merge_message_source() {
        assert_eq!(
            merge_source("Merge branch 'feature/login'\n").as_deref(),
            Some("feature/login")
        );
        assert_eq!(
            merge_source("Merge remote-tracking branch 'origin/x' into main").as_deref(),
            Some("origin/x")
        );
    }

    #[test]
    fn resolves_real_merge_conflicts() {
        use crate::git::model::{load_refs, load_status};
        let dir = std::env::temp_dir().join(format!("gitr-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| cmd::run(&dir, args);
        git(&["init", "-q", "-b", "main"]).unwrap();
        git(&["config", "user.email", "t@example.com"]).unwrap();
        git(&["config", "user.name", "T"]).unwrap();
        std::fs::write(dir.join("both.txt"), "top\nshared\nbottom\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "keep me\n").unwrap();
        git(&["add", "."]).unwrap();
        git(&["commit", "-qm", "base"]).unwrap();
        git(&["checkout", "-qb", "feature"]).unwrap();
        std::fs::write(dir.join("both.txt"), "top\nfeature line\nbottom\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "changed on feature\n").unwrap();
        git(&["commit", "-qam", "feature"]).unwrap();
        git(&["checkout", "-q", "main"]).unwrap();
        std::fs::write(dir.join("both.txt"), "top\nmain line\nbottom\n").unwrap();
        git(&["rm", "-q", "gone.txt"]).unwrap();
        git(&["commit", "-qam", "main"]).unwrap();
        assert!(git(&["merge", "feature"]).is_err(), "merge must conflict");

        let status = load_status(&dir).unwrap();
        assert_eq!(
            status.conflicts.get("both.txt"),
            Some(&ConflictKind::BothModified)
        );
        assert_eq!(
            status.conflicts.get("gone.txt"),
            Some(&ConflictKind::DeletedByUs)
        );
        let refs = load_refs(&dir).unwrap();
        assert_eq!(refs.state, Some(RepoState::Merging));
        let sides = refs.sides.unwrap();
        assert_eq!(
            (sides.ours.as_str(), sides.theirs.as_str()),
            ("main", "feature")
        );
        assert!(
            prepared_message(&dir)
                .unwrap()
                .starts_with("Merge branch 'feature'")
        );

        // both.txt: keep both lines, ours first.
        let text = std::fs::read_to_string(dir.join("both.txt")).unwrap();
        let mut segs = parse(&text).unwrap();
        for s in &mut segs {
            if let Segment::Conflict(h) = s {
                h.choice = Some(Choice::OursThenTheirs);
            }
        }
        std::fs::write(dir.join("both.txt"), render(&segs).unwrap()).unwrap();
        git(&["add", "--", "both.txt"]).unwrap();
        // gone.txt: take theirs (the modified file) over our deletion.
        for args in take_side_commands("gone.txt", ConflictKind::DeletedByUs, false) {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            git(&refs).unwrap();
        }

        assert!(load_status(&dir).unwrap().conflicts.is_empty());
        assert_eq!(
            git(&["show", ":both.txt"]).unwrap(),
            "top\nmain line\nfeature line\nbottom\n"
        );
        assert_eq!(git(&["show", ":gone.txt"]).unwrap(), "changed on feature\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn squash_message_is_tidy() {
        let draft = "Squashed commit of the following:\n\ncommit cd43684a\nAuthor: T <t@e.com>\nDate:   Wed Oct 7\n\n    login validation\n    \n    Longer explanation.\n\ncommit e155a786\nAuthor: T <t@e.com>\nDate:   Wed Oct 7\n\n    login form\n";
        assert_eq!(
            squash_message(draft, Some("feature/login")),
            "Squash merge branch 'feature/login'\n\n* login form\n* login validation"
        );
        assert!(
            squash_message(draft, Some("cd43684a674e82428831d6bb7485d530a224a4dc"))
                .starts_with("Squash merge commit 'cd43684'\n")
        );
        assert!(squash_message(draft, None).starts_with("Squash merge\n"));
        // A single squashed commit keeps its own subject.
        let one = "Squashed commit of the following:\n\ncommit e155a786\nAuthor: T\nDate:   x\n\n    login form\n";
        assert_eq!(squash_message(one, Some("feature/login")), "login form");
    }

    /// git's real squash draft is turned into the tidy message.
    #[test]
    fn squash_message_from_real_git_draft() {
        let dir = std::env::temp_dir().join(format!("gitr-squash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| cmd::run(&dir, args).unwrap();
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        git(&["checkout", "-qb", "feature/login"]);
        for (file, subject) in [("b.txt", "login form"), ("c.txt", "login validation")] {
            std::fs::write(dir.join(file), "x").unwrap();
            git(&["add", "."]);
            git(&[
                "commit",
                "-qm",
                subject,
                "-m",
                "Body text that must not appear.",
            ]);
        }
        git(&["checkout", "-q", "main"]);
        assert!(squash_draft(&dir).is_none());
        git(&["merge", "-q", "--squash", "feature/login"]);

        let draft = squash_draft(&dir).expect("git writes SQUASH_MSG");
        assert_eq!(
            squash_message(&draft, Some("feature/login")),
            "Squash merge branch 'feature/login'\n\n* login form\n* login validation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
