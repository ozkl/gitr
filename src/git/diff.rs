//! Unified diff parsing and partial patch construction (hunk / line staging).

use std::collections::BTreeSet;
use std::path::Path;

use super::cmd;
use super::model::FileChange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file`
    NoNewline,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default)]
pub struct FileDiff {
    /// `diff --git`, `index`, mode lines, `---`, `+++`.
    pub header: Vec<String>,
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    /// Synthesized diff for an untracked file; hunks cannot be applied as patches.
    pub untracked: bool,
    pub added: usize,
    pub removed: usize,
    /// Shown instead of the diff (e.g. file too large).
    pub notice: Option<String>,
}

/// Files larger than this are not loaded into the diff view.
pub const MAX_DIFF_BYTES: usize = 20 * 1024 * 1024;

pub fn too_large_notice(bytes: usize) -> String {
    format!("File too large to display ({})", crate::format::human_size(bytes as u64))
}

fn parse_range(s: &str) -> u32 {
    s.split(',').next().and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// Parses `git diff` output that contains (at most) a single file.
pub fn parse(text: &str) -> FileDiff {
    let mut diff = FileDiff::default();
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for raw in text.lines() {
        if let Some(rest) = raw.strip_prefix("@@") {
            // @@ -a,b +c,d @@ section
            let mut parts = rest.split_whitespace();
            let old = parts.next().unwrap_or("-0").trim_start_matches('-');
            let new = parts.next().unwrap_or("+0").trim_start_matches('+');
            old_no = parse_range(old);
            new_no = parse_range(new);
            diff.hunks.push(Hunk {
                header: raw.to_owned(),
                old_start: old_no,
                new_start: new_no,
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else {
            if raw.starts_with("Binary files") || raw.starts_with("GIT binary patch") {
                diff.binary = true;
            }
            diff.header.push(raw.to_owned());
            continue;
        };
        let (kind, text) = match raw.as_bytes().first() {
            Some(b'+') => (LineKind::Added, &raw[1..]),
            Some(b'-') => (LineKind::Removed, &raw[1..]),
            Some(b'\\') => (LineKind::NoNewline, raw),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            None => (LineKind::Context, ""),
            _ => (LineKind::Context, raw),
        };
        let line = match kind {
            LineKind::Context => {
                old_no += 1;
                new_no += 1;
                DiffLine { kind, old_no: Some(old_no - 1), new_no: Some(new_no - 1), text: text.to_owned() }
            }
            LineKind::Added => {
                new_no += 1;
                diff.added += 1;
                DiffLine { kind, old_no: None, new_no: Some(new_no - 1), text: text.to_owned() }
            }
            LineKind::Removed => {
                old_no += 1;
                diff.removed += 1;
                DiffLine { kind, old_no: Some(old_no - 1), new_no: None, text: text.to_owned() }
            }
            LineKind::NoNewline => DiffLine { kind, old_no: None, new_no: None, text: text.to_owned() },
        };
        hunk.lines.push(line);
    }
    diff
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

/// Builds a diff showing an untracked file as entirely added.
pub fn untracked_file_diff(repo: &Path, path: &str) -> FileDiff {
    let full = repo.join(path);
    let mut diff = FileDiff { untracked: true, ..Default::default() };
    let Ok(bytes) = std::fs::read(&full) else {
        if full.is_dir() {
            diff.header.push("Directory (possibly a nested repository)".into());
        }
        return diff;
    };
    if looks_binary(&bytes) {
        diff.binary = true;
        return diff;
    }
    if bytes.len() > MAX_DIFF_BYTES {
        diff.notice = Some(too_large_notice(bytes.len()));
        return diff;
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<DiffLine> = text
        .lines()
        .enumerate()
        .map(|(i, l)| DiffLine {
            kind: LineKind::Added,
            old_no: None,
            new_no: Some(i as u32 + 1),
            text: l.to_owned(),
        })
        .collect();
    diff.added = lines.len();
    if !lines.is_empty() {
        diff.hunks.push(Hunk {
            header: format!("@@ -0,0 +1,{} @@", lines.len()),
            old_start: 0,
            new_start: 1,
            lines,
        });
    }
    diff
}

fn diff_args(context: u32, ignore_whitespace: bool) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "diff".into(),
        "--no-ext-diff".into(),
        "--no-color".into(),
        "-M".into(),
        format!("-U{context}"),
    ];
    if ignore_whitespace {
        args.push("-w".into());
    }
    args
}

fn path_args(file: &FileChange) -> Vec<String> {
    let mut args = vec!["--".to_owned()];
    if let Some(old) = &file.old_path {
        args.push(old.clone());
    }
    args.push(file.path.clone());
    args
}

fn run_diff(repo: &Path, args: Vec<String>) -> Result<FileDiff, String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = cmd::run(repo, &refs)?;
    if out.len() > MAX_DIFF_BYTES {
        return Ok(FileDiff { notice: Some(format!("Diff too large to display ({})", crate::format::human_size(out.len() as u64))), ..Default::default() });
    }
    Ok(parse(&out))
}

/// Diff of a file in the working tree (`staged == false`) or index (`staged == true`).
pub fn working_diff(
    repo: &Path,
    file: &FileChange,
    staged: bool,
    context: u32,
    ignore_whitespace: bool,
) -> Result<FileDiff, String> {
    if !staged && file.kind == super::model::ChangeKind::Untracked {
        return Ok(untracked_file_diff(repo, &file.path));
    }
    let mut args = diff_args(context, ignore_whitespace);
    if staged {
        args.push("--cached".into());
    }
    args.extend(path_args(file));
    run_diff(repo, args)
}

/// Diff of a file introduced by a commit (against its first parent).
pub fn commit_file_diff(
    repo: &Path,
    id: &str,
    parent: Option<&str>,
    file: &FileChange,
    context: u32,
    ignore_whitespace: bool,
) -> Result<FileDiff, String> {
    let mut args = diff_args(context, ignore_whitespace);
    match parent {
        Some(p) => {
            args.push(p.to_owned());
            args.push(id.to_owned());
        }
        None => {
            // Root commit: diff against the empty tree.
            args.push("4b825dc642cb6eb9a060e54bf8d69288fbee4904".into());
            args.push(id.to_owned());
        }
    }
    args.extend(path_args(file));
    run_diff(repo, args)
}

/// Which lines to include in a partial patch.
#[derive(Debug, Clone)]
pub enum Selection {
    Hunk(usize),
    /// Hunk index and the indices (into `hunk.lines`) of selected lines.
    Lines(usize, BTreeSet<usize>),
}

/// Builds a patch containing part of `diff`.
///
/// `reverse` must be set when the patch will be applied with `--reverse`
/// (unstaging from the index, discarding from the working tree); it changes
/// how unselected lines are treated so that they are left untouched.
pub fn build_patch(diff: &FileDiff, selection: &Selection, reverse: bool) -> Option<String> {
    let (hunk_idx, selected) = match selection {
        Selection::Hunk(i) => (*i, None),
        Selection::Lines(i, lines) => (*i, Some(lines)),
    };
    let hunk = diff.hunks.get(hunk_idx)?;
    let mut body = String::new();
    let (mut old_count, mut new_count) = (0u32, 0u32);
    let mut last_emitted = false;
    let mut any_change = false;
    for (i, line) in hunk.lines.iter().enumerate() {
        let is_selected = selected.is_none_or(|s| s.contains(&i));
        match line.kind {
            LineKind::Context => {
                body.push(' ');
                body.push_str(&line.text);
                body.push('\n');
                old_count += 1;
                new_count += 1;
                last_emitted = true;
            }
            LineKind::Added | LineKind::Removed => {
                let added = line.kind == LineKind::Added;
                if is_selected {
                    body.push(if added { '+' } else { '-' });
                    if added { new_count += 1 } else { old_count += 1 }
                    any_change = true;
                    last_emitted = true;
                } else if added == reverse {
                    // Line exists on the side the patch applies to: keep it as context.
                    body.push(' ');
                    old_count += 1;
                    new_count += 1;
                    last_emitted = true;
                } else {
                    last_emitted = false;
                    continue;
                }
                body.push_str(&line.text);
                body.push('\n');
            }
            LineKind::NoNewline => {
                if last_emitted {
                    body.push_str(&line.text);
                    body.push('\n');
                }
            }
        }
    }
    if !any_change {
        return None;
    }
    let mut patch = String::new();
    for h in &diff.header {
        if h.starts_with("diff --git")
            || h.starts_with("---")
            || h.starts_with("+++")
            || h.starts_with("new file mode")
            || h.starts_with("deleted file mode")
            || h.starts_with("old mode")
            || h.starts_with("new mode")
        {
            patch.push_str(h);
            patch.push('\n');
        }
    }
    patch.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        hunk.old_start, old_count, hunk.new_start, new_count
    ));
    patch.push_str(&body);
    Some(patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/f.txt b/f.txt
index 1111111..2222222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,4 +1,4 @@
 one
-two
+TWO
+extra
 three
-four
";

    #[test]
    fn parses_hunks_and_line_numbers() {
        let d = parse(SAMPLE);
        assert_eq!(d.header.len(), 4);
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!(h.lines.len(), 6);
        assert_eq!(h.lines[1].old_no, Some(2));
        assert_eq!(h.lines[2].new_no, Some(2));
        assert_eq!(h.lines[4].old_no, Some(3));
        assert_eq!(h.lines[4].new_no, Some(4));
        assert_eq!((d.added, d.removed), (2, 2));
    }

    #[test]
    fn partial_forward_patch_keeps_unselected_removals_as_context() {
        let d = parse(SAMPLE);
        // Select only "+TWO" (index 2).
        let p = build_patch(&d, &Selection::Lines(0, [2].into()), false).unwrap();
        assert!(p.contains("@@ -1,4 +1,5 @@"));
        assert!(p.contains(" two\n+TWO\n three\n four\n"));
        assert!(!p.contains("extra"));
    }

    #[test]
    fn partial_reverse_patch_keeps_unselected_additions_as_context() {
        let d = parse(SAMPLE);
        // Select only "-four" (index 5).
        let p = build_patch(&d, &Selection::Lines(0, [5].into()), true).unwrap();
        assert!(p.contains(" one\n TWO\n extra\n three\n-four\n"));
    }

    #[test]
    fn stages_and_unstages_selected_lines_with_real_git() {
        use crate::git::{ChangeKind, FileChange};
        let dir = std::env::temp_dir().join(format!("gitr-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| cmd::run(&dir, args).unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "init"]);
        std::fs::write(dir.join("f.txt"), "one\nTWO\nthree\nfour\n").unwrap();

        // Stage only the "+four" line.
        let file = FileChange { path: "f.txt".into(), old_path: None, kind: ChangeKind::Modified };
        let d = working_diff(&dir, &file, false, 3, false).unwrap();
        let idx = d.hunks[0].lines.iter().position(|l| l.text == "four").unwrap();
        let patch = build_patch(&d, &Selection::Lines(0, [idx].into()), false).unwrap();
        cmd::run_with_input(&dir, &["apply", "--cached", "--recount", "-"], patch.as_bytes()).unwrap();
        assert_eq!(git(&["show", ":f.txt"]), "one\ntwo\nthree\nfour\n");

        // Unstage it again with a reverse patch of the staged diff.
        let staged = working_diff(&dir, &file, true, 3, false).unwrap();
        let patch = build_patch(&staged, &Selection::Hunk(0), true).unwrap();
        cmd::run_with_input(&dir, &["apply", "--cached", "--reverse", "--recount", "-"], patch.as_bytes()).unwrap();
        assert_eq!(git(&["show", ":f.txt"]), "one\ntwo\nthree\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
