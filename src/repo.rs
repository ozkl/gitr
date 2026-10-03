//! Per-repository state. All git calls run on background threads; results come back as [`Msg`]s.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::git::diff::{self, FileDiff, Selection};
use crate::git::graph::{self, GraphRow};
use crate::git::{self, Commit, CommitDetails, FileChange, Refs, Status, cmd};

pub type Task = Box<dyn FnOnce(&Path) -> Result<String, String> + Send>;

#[derive(Debug, Clone)]
pub enum Loaded<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> From<Result<T, String>> for Loaded<T> {
    fn from(r: Result<T, String>) -> Self {
        match r {
            Ok(v) => Loaded::Ready(v),
            Err(e) => Loaded::Failed(e),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiffKey {
    Work { path: String, staged: bool },
    Commit { id: String, path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Generic,
    Commit,
    /// Like `Generic`, but success is reported with a notification showing the output.
    Notify,
}

enum Msg {
    Log(u64, Result<(Vec<Commit>, Vec<GraphRow>), String>),
    Refs(u64, Result<Refs, String>),
    Status(u64, Result<Status, String>),
    Details(String, Result<CommitDetails, String>),
    Diff(DiffKey, u64, Result<FileDiff, String>),
    Tree(String, Result<(Vec<String>, HashSet<String>), String>),
    Blob(String, String, Result<Vec<u8>, String>),
    Images(String, crate::preview::ImagePair),
    History(String, String, Result<Vec<git::HistoryEntry>, String>),
    HistoryDiff(String, String, Result<FileDiff, String>),
    OpStarted(String),
    OpDone {
        label: String,
        kind: OpKind,
        result: Result<String, String>,
    },
}

struct Job {
    label: String,
    kind: OpKind,
    task: Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    LocalChanges,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailTab {
    Commit,
    Changes,
    FileTree,
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub label: String,
    pub ok: bool,
    pub output: String,
    pub time: chrono::DateTime<chrono::Local>,
}

/// Built-in three-way merge of one conflicted file.
pub struct MergeEditor {
    pub path: String,
    pub segments: Vec<git::conflict::Segment>,
    /// Conflict to scroll to on the next frame.
    pub scroll_to: Option<usize>,
}

/// State of the file history window.
pub struct FileHistory {
    pub path: String,
    /// Revision the history starts from.
    pub rev: String,
    pub entries: Loaded<Vec<git::HistoryEntry>>,
    pub selected: Option<usize>,
    /// Diff of the file in the selected commit: (commit id, diff).
    pub diff: Option<(String, Loaded<FileDiff>)>,
    pub scroll_to_selected: bool,
}

pub struct Notice {
    pub title: String,
    pub text: String,
    pub error: bool,
}

pub struct RepoTab {
    pub path: PathBuf,
    pub name: String,
    ctx: egui::Context,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    jobs: Sender<Job>,

    pub commits: Vec<Commit>,
    pub graph: Vec<GraphRow>,
    pub index_of: HashMap<String, usize>,
    pub refs: Refs,
    pub status: Status,
    pub log_loaded: bool,
    pub load_error: Option<String>,
    log_gen: u64,
    refs_gen: u64,
    status_gen: u64,
    diff_gen: u64,

    pub view: View,
    pub selected: Option<String>,
    pub scroll_to_selected: bool,
    pub details: Option<Loaded<CommitDetails>>,
    pub detail_tab: DetailTab,
    pub commit_file: Option<String>,
    pub commit_diffs: HashMap<String, Loaded<FileDiff>>,
    pub expanded_files: BTreeSet<String>,
    pub tree: Option<(String, Loaded<Vec<String>>)>,
    /// LFS-tracked paths of `tree`.
    pub tree_lfs: HashSet<String>,
    pub tree_file: Option<(String, Loaded<Vec<u8>>)>,
    /// Decoded image previews by key (see [`RepoTab::request_images`]).
    pub image_previews: HashMap<String, Loaded<crate::preview::ImagePair>>,
    /// Open file history window, if any.
    pub file_history: Option<FileHistory>,
    pub merge_editor: Option<MergeEditor>,
    /// The prepared merge message was put into the commit box for the current operation.
    merge_message_loaded: bool,

    pub selected_change: Option<(bool, String)>,
    pub change_diff: Option<(DiffKey, Loaded<FileDiff>)>,
    pub line_selection: BTreeSet<(usize, usize)>,
    /// Collapsed folders in the tree view of local changes, keyed by (staged, folder path).
    pub collapsed_dirs: HashSet<(bool, String)>,
    /// All selected files in the list of `selected_change` (which is the focused one whose diff is shown).
    pub selected_changes: BTreeSet<String>,
    /// Anchor for shift-click range selection.
    pub selection_anchor: Option<String>,
    /// File paths in display order, per list (index 0 = unstaged, 1 = staged), from the last frame.
    pub change_order: [Vec<String>; 2],
    /// Path filter for the local changes lists (applies to both).
    pub changes_filter: String,
    /// Request to focus the filter field on the next frame.
    pub focus_changes_filter: bool,
    pub last_clicked_line: Option<(usize, usize)>,
    pub commit_subject: String,
    pub commit_body: String,
    pub amend: bool,

    pub search: String,
    pub sidebar_filter: String,
    pub diff_context: u32,
    pub ignore_whitespace: bool,

    pub running: Vec<String>,
    pub activity: Vec<Activity>,
    pub notices: Vec<Notice>,
    last_status_poll: Instant,
    pub commit_limit: usize,
}

impl RepoTab {
    pub fn open(path: PathBuf, ctx: egui::Context, commit_limit: usize) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let (tx, rx) = channel();
        let (jobs, job_rx) = channel::<Job>();
        {
            // Mutating operations run one at a time, in order, on a dedicated thread.
            let tx = tx.clone();
            let ctx = ctx.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                while let Ok(job) = job_rx.recv() {
                    let _ = tx.send(Msg::OpStarted(job.label.clone()));
                    ctx.request_repaint();
                    let result = (job.task)(&path);
                    let _ = tx.send(Msg::OpDone {
                        label: job.label,
                        kind: job.kind,
                        result,
                    });
                    ctx.request_repaint();
                }
            });
        }
        let mut tab = Self {
            path,
            name,
            ctx,
            tx,
            rx,
            jobs,
            commits: Vec::new(),
            graph: Vec::new(),
            index_of: HashMap::new(),
            refs: Refs::default(),
            status: Status::default(),
            log_loaded: false,
            load_error: None,
            log_gen: 0,
            refs_gen: 0,
            status_gen: 0,
            diff_gen: 0,
            view: View::History,
            selected: None,
            scroll_to_selected: false,
            details: None,
            detail_tab: DetailTab::Commit,
            commit_file: None,
            commit_diffs: HashMap::new(),
            expanded_files: BTreeSet::new(),
            tree: None,
            tree_lfs: HashSet::new(),
            tree_file: None,
            image_previews: HashMap::new(),
            file_history: None,
            merge_editor: None,
            merge_message_loaded: false,
            selected_change: None,
            change_diff: None,
            line_selection: BTreeSet::new(),
            collapsed_dirs: HashSet::new(),
            selected_changes: BTreeSet::new(),
            selection_anchor: None,
            change_order: [Vec::new(), Vec::new()],
            changes_filter: String::new(),
            focus_changes_filter: false,
            last_clicked_line: None,
            commit_subject: String::new(),
            commit_body: String::new(),
            amend: false,
            search: String::new(),
            sidebar_filter: String::new(),
            diff_context: 3,
            ignore_whitespace: false,
            running: Vec::new(),
            activity: Vec::new(),
            notices: Vec::new(),
            last_status_poll: Instant::now(),
            commit_limit,
        };
        tab.refresh();
        tab
    }

    fn spawn<F>(&self, f: F)
    where
        F: FnOnce(&Path) -> Msg + Send + 'static,
    {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let path = self.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f(&path));
            ctx.request_repaint();
        });
    }

    pub fn is_busy(&self) -> bool {
        !self.running.is_empty()
    }

    // ------------------------------------------------------------------
    // Loading

    pub fn refresh(&mut self) {
        self.reload_log();
        self.reload_refs();
        self.reload_status();
    }

    pub fn reload_log(&mut self) {
        self.log_gen += 1;
        let generation = self.log_gen;
        let limit = self.commit_limit;
        self.spawn(move |p| {
            Msg::Log(
                generation,
                git::load_log(p, limit).map(|commits| {
                    let rows = graph::build(&commits);
                    (commits, rows)
                }),
            )
        });
    }

    pub fn reload_refs(&mut self) {
        self.refs_gen += 1;
        let generation = self.refs_gen;
        self.spawn(move |p| Msg::Refs(generation, git::load_refs(p)));
    }

    pub fn reload_status(&mut self) {
        self.status_gen += 1;
        let generation = self.status_gen;
        self.last_status_poll = Instant::now();
        self.spawn(move |p| Msg::Status(generation, git::load_status(p)));
    }

    /// Called every frame; polls the working tree while the window is focused.
    pub fn tick(&mut self, focused: bool) {
        self.process_messages();
        if focused && !self.is_busy() && self.last_status_poll.elapsed() > Duration::from_secs(3) {
            self.reload_status();
        }
        if focused {
            self.ctx.request_repaint_after(Duration::from_secs(3));
        }
    }

    fn process_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Log(generation, result) if generation == self.log_gen => match result {
                    Ok((commits, rows)) => {
                        self.index_of = commits
                            .iter()
                            .enumerate()
                            .map(|(i, c)| (c.id.clone(), i))
                            .collect();
                        self.commits = commits;
                        self.graph = rows;
                        let first_load = !self.log_loaded;
                        self.log_loaded = true;
                        self.load_error = None;
                        // Nothing to browse yet: start where the first commit is made.
                        if first_load && self.commits.is_empty() {
                            self.view = View::LocalChanges;
                        }
                        if self.selected.is_none() && self.view == View::History {
                            if let Some(first) = self.commits.first().map(|c| c.id.clone()) {
                                self.select_commit(first);
                            }
                        } else if let Some(sel) = &self.selected {
                            if !self.index_of.contains_key(sel) {
                                self.selected = None;
                                self.details = None;
                            }
                        }
                    }
                    Err(e) => {
                        self.log_loaded = true;
                        self.load_error = Some(e);
                    }
                },
                Msg::Refs(generation, result) if generation == self.refs_gen => match result {
                    Ok(refs) => {
                        self.refs = refs;
                        self.prefill_merge_message();
                    }
                    Err(e) => self.load_error = Some(e),
                },
                Msg::Status(generation, result) if generation == self.status_gen => {
                    if let Ok(status) = result {
                        let changed = !same_status(&status, &self.status);
                        self.status = status;
                        if changed {
                            self.on_status_changed();
                        }
                    }
                }
                Msg::Details(id, result) => {
                    if self.selected.as_deref() == Some(id.as_str()) {
                        self.details = Some(result.into());
                    }
                }
                Msg::Diff(key, generation, result) => match &key {
                    DiffKey::Work { .. } => {
                        if generation == self.diff_gen
                            && self.change_diff.as_ref().is_some_and(|(k, _)| *k == key)
                        {
                            self.change_diff = Some((key, result.into()));
                        }
                    }
                    DiffKey::Commit { id, path } => {
                        if self.selected.as_deref() == Some(id.as_str()) {
                            self.commit_diffs.insert(path.clone(), result.into());
                        }
                    }
                },
                Msg::Tree(id, result) => {
                    if self.tree.as_ref().is_some_and(|(t, _)| *t == id) {
                        let result = result.map(|(paths, lfs)| {
                            self.tree_lfs = lfs;
                            paths
                        });
                        self.tree = Some((id, result.into()));
                    }
                }
                Msg::Blob(id, path, result) => {
                    if self.selected.as_deref() == Some(id.as_str())
                        && self.tree_file.as_ref().is_some_and(|(p, _)| *p == path)
                    {
                        self.tree_file = Some((path, result.into()));
                    }
                }
                Msg::Images(key, pair) => {
                    if self.image_previews.contains_key(&key) {
                        self.image_previews.insert(key, Loaded::Ready(pair));
                    }
                }
                Msg::History(path, rev, result) => {
                    if let Some(h) = self
                        .file_history
                        .as_mut()
                        .filter(|h| h.path == path && h.rev == rev)
                    {
                        let first = result.as_ref().ok().filter(|e| !e.is_empty()).map(|_| 0);
                        h.entries = result.into();
                        if h.selected.is_none() {
                            if let Some(i) = first {
                                self.select_history_entry(i);
                            }
                        }
                    }
                }
                Msg::HistoryDiff(id, path, result) => {
                    if let Some(h) = self.file_history.as_mut().filter(|h| h.path == path) {
                        if h.diff.as_ref().is_some_and(|(d, _)| *d == id) {
                            h.diff = Some((id, result.into()));
                        }
                    }
                }
                Msg::OpStarted(label) => self.running.push(label),
                Msg::OpDone {
                    label,
                    kind,
                    result,
                } => {
                    if let Some(i) = self.running.iter().position(|l| *l == label) {
                        self.running.remove(i);
                    }
                    let ok = result.is_ok();
                    let output = match &result {
                        Ok(s) | Err(s) => truncate_output(s.trim()),
                    };
                    self.activity.push(Activity {
                        label: label.clone(),
                        ok,
                        output: output.clone(),
                        time: chrono::Local::now(),
                    });
                    if !ok {
                        self.notices.push(Notice {
                            title: format!("{label} failed"),
                            text: output,
                            error: true,
                        });
                    } else if kind == OpKind::Commit {
                        self.commit_subject.clear();
                        self.commit_body.clear();
                        self.amend = false;
                    } else if kind == OpKind::Notify {
                        self.notices.push(Notice {
                            title: label.clone(),
                            text: output,
                            error: false,
                        });
                    }
                    self.line_selection.clear();
                    self.refresh();
                    self.reload_change_diff();
                }
                _ => {} // stale result
            }
        }
    }

    fn on_status_changed(&mut self) {
        // Keep the selected file if it is still present, preferring the same list.
        if let Some((staged, path)) = self.selected_change.clone() {
            let in_list = |list: &Vec<FileChange>, p: &str| list.iter().any(|f| f.path == p);
            let same = if staged {
                &self.status.staged
            } else {
                &self.status.unstaged
            };
            let other = if staged {
                &self.status.unstaged
            } else {
                &self.status.staged
            };
            if in_list(same, &path) {
                let same = same.clone();
                self.selected_changes.retain(|p| in_list(&same, p));
                self.reload_change_diff();
            } else if in_list(other, &path) {
                // The selection moved to the other list (staged/unstaged): follow it.
                let other = other.clone();
                let keep: BTreeSet<String> = self
                    .selected_changes
                    .iter()
                    .filter(|p| in_list(&other, p))
                    .cloned()
                    .collect();
                self.select_change(!staged, path);
                self.selected_changes.extend(keep);
            } else {
                self.selected_change = None;
                self.selected_changes.clear();
                self.change_diff = None;
            }
        }
        // Close the merge editor once its file is no longer conflicted.
        if self
            .merge_editor
            .as_ref()
            .is_some_and(|e| !self.status.conflicts.contains_key(&e.path))
        {
            self.merge_editor = None;
        }
    }

    // ------------------------------------------------------------------
    // Selection

    pub fn select_commit(&mut self, id: String) {
        self.view = View::History;
        if self.selected.as_deref() == Some(id.as_str()) && self.details.is_some() {
            return;
        }
        self.selected = Some(id.clone());
        self.details = Some(Loaded::Loading);
        self.commit_file = None;
        self.commit_diffs.clear();
        self.expanded_files.clear();
        self.tree = None;
        self.tree_file = None;
        let id2 = id.clone();
        self.spawn(move |p| Msg::Details(id2.clone(), git::load_commit_details(p, &id2)));
        if self.detail_tab == DetailTab::FileTree {
            self.load_tree();
        }
    }

    pub fn reveal_commit(&mut self, id: &str) {
        let full = self
            .commits
            .iter()
            .find(|c| c.id.starts_with(id))
            .map(|c| c.id.clone());
        if let Some(full) = full {
            self.select_commit(full);
            self.scroll_to_selected = true;
        }
    }

    pub fn load_commit_diff(&mut self, file: &FileChange) {
        let Some(Loaded::Ready(details)) = &self.details else {
            return;
        };
        if self.commit_diffs.contains_key(&file.path) {
            return;
        }
        let id = details.id.clone();
        let parent = details.parents.first().cloned();
        let file = file.clone();
        let context = self.diff_context;
        let ws = self.ignore_whitespace;
        self.commit_diffs.insert(file.path.clone(), Loaded::Loading);
        self.spawn(move |p| {
            let result = diff::commit_file_diff(p, &id, parent.as_deref(), &file, context, ws);
            Msg::Diff(
                DiffKey::Commit {
                    id,
                    path: file.path,
                },
                0,
                result,
            )
        });
    }

    pub fn reload_commit_diffs(&mut self) {
        let Some(Loaded::Ready(details)) = &self.details else {
            return;
        };
        let files: Vec<FileChange> = details
            .files
            .iter()
            .filter(|f| self.commit_diffs.contains_key(&f.path))
            .cloned()
            .collect();
        self.commit_diffs.clear();
        for f in files {
            self.load_commit_diff(&f);
        }
    }

    pub fn load_tree(&mut self) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.tree.as_ref().is_some_and(|(t, _)| *t == id) {
            return;
        }
        self.tree = Some((id.clone(), Loaded::Loading));
        self.spawn(move |p| Msg::Tree(id.clone(), git::list_tree(p, &id)));
    }

    pub fn load_tree_file(&mut self, path: String) {
        let Some(id) = self.selected.clone() else {
            return;
        };
        self.tree_file = Some((path.clone(), Loaded::Loading));
        self.spawn(move |p| Msg::Blob(id.clone(), path.clone(), git::file_at(p, &id, &path)));
    }

    /// Selects a single file and shows its diff.
    pub fn select_change(&mut self, staged: bool, path: String) {
        self.selected_changes = BTreeSet::from([path.clone()]);
        self.selection_anchor = Some(path.clone());
        self.selected_change = Some((staged, path));
        self.line_selection.clear();
        self.last_clicked_line = None;
        self.change_diff = None;
        self.reload_change_diff();
    }

    /// Click handling with ⌘/Ctrl (toggle) and Shift (range) modifiers.
    pub fn click_change(&mut self, staged: bool, path: String, modifiers: egui::Modifiers) {
        let same_list = self
            .selected_change
            .as_ref()
            .is_some_and(|(s, _)| *s == staged);
        if !same_list || !(modifiers.command || modifiers.shift) {
            self.select_change(staged, path);
            return;
        }
        if modifiers.shift {
            let order = &self.change_order[usize::from(staged)];
            let anchor = self
                .selection_anchor
                .clone()
                .unwrap_or_else(|| path.clone());
            let (a, b) = (
                order.iter().position(|p| *p == anchor),
                order.iter().position(|p| *p == path),
            );
            if let (Some(a), Some(b)) = (a, b) {
                let (lo, hi) = (a.min(b), a.max(b));
                if !modifiers.command {
                    self.selected_changes.clear();
                }
                self.selected_changes.extend(order[lo..=hi].iter().cloned());
            }
            self.focus_change(staged, path);
        } else if self.selected_changes.contains(&path) {
            self.selected_changes.remove(&path);
            if self
                .selected_change
                .as_ref()
                .is_some_and(|(_, p)| *p == path)
            {
                if let Some(next) = self.selected_changes.iter().next().cloned() {
                    self.focus_change(staged, next);
                } else {
                    self.selected_change = None;
                    self.change_diff = None;
                }
            }
        } else {
            self.selected_changes.insert(path.clone());
            self.selection_anchor = Some(path.clone());
            self.focus_change(staged, path);
        }
    }

    /// Changes the focused file without touching the multi-selection.
    fn focus_change(&mut self, staged: bool, path: String) {
        if self.selected_change.as_ref() != Some(&(staged, path.clone())) {
            self.selected_change = Some((staged, path));
            self.line_selection.clear();
            self.last_clicked_line = None;
            self.change_diff = None;
            self.reload_change_diff();
        }
    }

    /// Selects a group of files (e.g. everything in a folder); `add` keeps the current selection.
    pub fn select_changes(&mut self, staged: bool, paths: Vec<String>, add: bool) {
        let Some(first) = paths.first().cloned() else {
            return;
        };
        let same_list = self
            .selected_change
            .as_ref()
            .is_some_and(|(s, _)| *s == staged);
        if add && same_list {
            self.selected_changes.extend(paths);
        } else {
            self.select_change(staged, first);
            self.selected_changes = paths.into_iter().collect();
        }
    }

    pub fn is_change_selected(&self, staged: bool, path: &str) -> bool {
        self.selected_change
            .as_ref()
            .is_some_and(|(s, _)| *s == staged)
            && self.selected_changes.contains(path)
    }

    /// Selected files of one list, in list order.
    pub fn selected_files(&self, staged: bool) -> Vec<FileChange> {
        if !self
            .selected_change
            .as_ref()
            .is_some_and(|(s, _)| *s == staged)
        {
            return Vec::new();
        }
        let list = if staged {
            &self.status.staged
        } else {
            &self.status.unstaged
        };
        list.iter()
            .filter(|f| self.selected_changes.contains(&f.path))
            .cloned()
            .collect()
    }

    /// Whether a path passes the local changes filter (case-insensitive substring).
    pub fn change_matches(&self, path: &str) -> bool {
        let q = self.changes_filter.trim();
        q.is_empty() || path.to_lowercase().contains(&q.to_lowercase())
    }

    /// Files of one list that pass the filter.
    pub fn visible_changes(&self, staged: bool) -> Vec<FileChange> {
        let list = if staged {
            &self.status.staged
        } else {
            &self.status.unstaged
        };
        list.iter()
            .filter(|f| self.change_matches(&f.path))
            .cloned()
            .collect()
    }

    /// Drops selected files hidden by the filter.
    pub fn apply_changes_filter(&mut self) {
        let keep: BTreeSet<String> = self
            .selected_changes
            .iter()
            .filter(|p| self.change_matches(p))
            .cloned()
            .collect();
        self.selected_changes = keep;
        if let Some((staged, path)) = self.selected_change.clone() {
            if !self.change_matches(&path) {
                match self.selected_changes.iter().next().cloned() {
                    Some(next) => self.focus_change(staged, next),
                    None => {
                        self.selected_change = None;
                        self.change_diff = None;
                    }
                }
            }
        }
    }

    pub fn select_all_changes(&mut self, staged: bool) {
        let list = self.visible_changes(staged);
        let Some(first) = list.first().map(|f| f.path.clone()) else {
            return;
        };
        let all: BTreeSet<String> = list.iter().map(|f| f.path.clone()).collect();
        let focus = match &self.selected_change {
            Some((s, p)) if *s == staged => p.clone(),
            _ => first,
        };
        self.focus_change(staged, focus);
        self.selected_changes = all;
    }

    /// Moves the focus up/down in the list with the current selection (arrow keys).
    pub fn move_change_selection(&mut self, delta: i32, extend: bool) {
        let Some((staged, path)) = self.selected_change.clone() else {
            return;
        };
        let order = &self.change_order[usize::from(staged)];
        let Some(i) = order.iter().position(|p| *p == path) else {
            return;
        };
        let j = (i as i64 + delta as i64).clamp(0, order.len() as i64 - 1) as usize;
        let next = order[j].clone();
        if extend {
            self.selected_changes.insert(next.clone());
            self.focus_change(staged, next);
        } else {
            self.select_change(staged, next);
        }
    }

    /// Puts git's prepared message (MERGE_MSG) into an empty commit box once per operation.
    fn prefill_merge_message(&mut self) {
        if self.refs.state.is_none() {
            self.merge_message_loaded = false;
            return;
        }
        if self.merge_message_loaded {
            return;
        }
        self.merge_message_loaded = true;
        if !self.commit_subject.trim().is_empty() {
            return;
        }
        if let Some(msg) = git::conflict::prepared_message(&self.path) {
            let (subject, body) = msg.split_once('\n').unwrap_or((&msg, ""));
            self.commit_subject = subject.trim().to_owned();
            self.commit_body = body.trim().to_owned();
        }
    }

    /// Resolves a conflicted file by taking our or their version entirely.
    pub fn take_conflict_side(&mut self, path: &str, ours: bool) {
        let Some(kind) = self.status.conflicts.get(path).copied() else {
            return;
        };
        let side = if ours { "local" } else { "remote" };
        self.merge_editor = None;
        self.git_seq(
            format!("Use {side} version of {path}"),
            git::conflict::take_side_commands(path, kind, ours),
        );
    }

    /// Opens the built-in merge editor for a conflicted text file.
    pub fn open_merge_editor(&mut self, path: &str) -> Result<(), String> {
        let bytes =
            std::fs::read(self.path.join(path)).map_err(|e| format!("Cannot read {path}: {e}"))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| "This file is not UTF-8 text; use an external merge tool.".to_owned())?;
        let segments = git::conflict::parse(&text)
            .ok_or_else(|| "No conflict markers found in the file.".to_owned())?;
        self.merge_editor = Some(MergeEditor {
            path: path.to_owned(),
            segments,
            scroll_to: Some(0),
        });
        Ok(())
    }

    /// Writes the merged content and marks the file resolved.
    pub fn save_merge(&mut self) {
        let Some(editor) = self.merge_editor.take() else {
            return;
        };
        let Some(content) = git::conflict::render(&editor.segments) else {
            self.merge_editor = Some(editor);
            return;
        };
        let path = editor.path;
        self.run_task(
            format!("Resolve {path}"),
            OpKind::Generic,
            Box::new(move |repo| {
                std::fs::write(repo.join(&path), content)
                    .map_err(|e| format!("Cannot write {path}: {e}"))?;
                cmd::run(repo, &["add", "--", &path])
            }),
        );
    }

    /// Opens the history window for `path`, starting at `rev`.
    pub fn open_file_history(&mut self, rev: &str, path: &str) {
        self.file_history = Some(FileHistory {
            path: path.to_owned(),
            rev: rev.to_owned(),
            entries: Loaded::Loading,
            selected: None,
            diff: None,
            scroll_to_selected: false,
        });
        let (rev, path) = (rev.to_owned(), path.to_owned());
        let limit = self.commit_limit;
        self.spawn(move |p| {
            let result = git::file_history(p, &rev, &path, limit);
            Msg::History(path, rev, result)
        });
    }

    pub fn select_history_entry(&mut self, index: usize) {
        let context = self.diff_context;
        let ws = self.ignore_whitespace;
        let Some(h) = self.file_history.as_mut() else {
            return;
        };
        let Loaded::Ready(entries) = &h.entries else {
            return;
        };
        let Some(entry) = entries.get(index).cloned() else {
            return;
        };
        h.selected = Some(index);
        h.scroll_to_selected = true;
        if crate::preview::is_image_path(&entry.file.path) {
            h.diff = None;
            return;
        }
        h.diff = Some((entry.commit.id.clone(), Loaded::Loading));
        let path = h.path.clone();
        self.spawn(move |p| {
            let parent = entry.commit.parents.first().cloned();
            let result = diff::commit_file_diff(
                p,
                &entry.commit.id,
                parent.as_deref(),
                &entry.file,
                context,
                ws,
            );
            Msg::HistoryDiff(entry.commit.id, path, result)
        });
    }

    /// Loads and decodes two versions of an image in the background, once per `key`.
    pub fn request_images(
        &mut self,
        key: &str,
        old: Option<crate::preview::Source>,
        new: Option<crate::preview::Source>,
    ) {
        if self.image_previews.contains_key(key) {
            return;
        }
        if self.image_previews.len() >= 32 {
            self.image_previews.clear();
        }
        self.image_previews.insert(key.to_owned(), Loaded::Loading);
        let key = key.to_owned();
        self.spawn(move |p| Msg::Images(key, crate::preview::load_pair(p, old, new)));
    }

    pub fn reload_change_diff(&mut self) {
        // Working-tree images may have changed on disk.
        self.image_previews.retain(|k, _| !k.starts_with("work:"));
        let Some((staged, path)) = self.selected_change.clone() else {
            return;
        };
        let list = if staged {
            &self.status.staged
        } else {
            &self.status.unstaged
        };
        let Some(file) = list.iter().find(|f| f.path == path).cloned() else {
            return;
        };
        let key = DiffKey::Work { path, staged };
        if !self.change_diff.as_ref().is_some_and(|(k, _)| *k == key) {
            self.change_diff = Some((key.clone(), Loaded::Loading));
        }
        self.diff_gen += 1;
        let generation = self.diff_gen;
        let context = self.diff_context;
        let ws = self.ignore_whitespace;
        self.spawn(move |p| {
            Msg::Diff(
                key,
                generation,
                diff::working_diff(p, &file, staged, context, ws),
            )
        });
    }

    pub fn change_file(&self, staged: bool, path: &str) -> Option<&FileChange> {
        let list = if staged {
            &self.status.staged
        } else {
            &self.status.unstaged
        };
        list.iter().find(|f| f.path == path)
    }

    // ------------------------------------------------------------------
    // Operations

    pub fn run_task(&mut self, label: impl Into<String>, kind: OpKind, task: Task) {
        let _ = self.jobs.send(Job {
            label: label.into(),
            kind,
            task,
        });
    }

    /// Queues `git <args>`.
    pub fn git(&mut self, label: impl Into<String>, args: Vec<String>) {
        self.run_task(
            label,
            OpKind::Generic,
            Box::new(move |p| {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                cmd::run_full(p, &refs).map(|o| format!("{}{}", o.stdout, o.stderr))
            }),
        );
    }

    /// Queues several git commands, stopping at the first failure.
    pub fn git_seq(&mut self, label: impl Into<String>, commands: Vec<Vec<String>>) {
        self.run_task(
            label,
            OpKind::Generic,
            Box::new(move |p| {
                let mut out = String::new();
                for args in commands {
                    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                    let o = cmd::run_full(p, &refs)?;
                    out.push_str(&o.stdout);
                    out.push_str(&o.stderr);
                }
                Ok(out)
            }),
        );
    }

    pub fn stage(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let mut args = vec!["add".into(), "-A".into(), "--".into()];
        args.extend(paths);
        self.git("Stage", args);
    }

    pub fn unstage(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        let has_head = self.refs.head_id.is_some();
        let mut args: Vec<String> = if has_head {
            vec!["reset".into(), "-q".into(), "HEAD".into(), "--".into()]
        } else {
            vec![
                "rm".into(),
                "--cached".into(),
                "-r".into(),
                "-q".into(),
                "--".into(),
            ]
        };
        args.extend(paths);
        self.git("Unstage", args);
    }

    pub fn discard(&mut self, files: Vec<FileChange>) {
        let (untracked, tracked): (Vec<_>, Vec<_>) = files
            .into_iter()
            .partition(|f| f.kind == git::ChangeKind::Untracked);
        let mut commands = Vec::new();
        if !tracked.is_empty() {
            let mut args: Vec<String> = vec!["checkout".into(), "--".into()];
            args.extend(tracked.into_iter().map(|f| f.path));
            commands.push(args);
        }
        if !untracked.is_empty() {
            let mut args: Vec<String> = vec!["clean".into(), "-f".into(), "-q".into(), "--".into()];
            args.extend(untracked.into_iter().map(|f| f.path));
            commands.push(args);
        }
        self.git_seq("Discard changes", commands);
    }

    /// Stage/unstage/discard part of the current working diff.
    pub fn apply_partial(&mut self, selection: Selection, action: PatchAction) {
        let Some((DiffKey::Work { staged, .. }, Loaded::Ready(d))) = &self.change_diff else {
            return;
        };
        let reverse = matches!(action, PatchAction::Unstage | PatchAction::Discard);
        debug_assert_eq!(*staged, action == PatchAction::Unstage);
        let Some(patch) = diff::build_patch(d, &selection, reverse) else {
            return;
        };
        let (label, args): (&str, &[&str]) = match action {
            PatchAction::Stage => ("Stage lines", &["apply", "--cached", "--recount", "-"]),
            PatchAction::Unstage => (
                "Unstage lines",
                &["apply", "--cached", "--reverse", "--recount", "-"],
            ),
            PatchAction::Discard => ("Discard lines", &["apply", "--reverse", "--recount", "-"]),
        };
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        self.run_task(
            label,
            OpKind::Generic,
            Box::new(move |p| {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                cmd::run_with_input(p, &refs, patch.as_bytes())
            }),
        );
    }

    pub fn commit(&mut self, and_push: bool) {
        let mut message = self.commit_subject.trim().to_owned();
        let body = self.commit_body.trim();
        if !body.is_empty() {
            message.push_str("\n\n");
            message.push_str(body);
        }
        let amend = self.amend;
        let push = and_push.then(|| self.push_args_for_head()).flatten();
        self.run_task(
            if amend { "Amend commit" } else { "Commit" },
            OpKind::Commit,
            Box::new(move |p| {
                let mut args = vec!["commit", "-q", "--cleanup=strip", "-F", "-"];
                if amend {
                    args.push("--amend");
                }
                let mut out = cmd::run_with_input(p, &args, message.as_bytes())?;
                if let Some(push) = push {
                    let refs: Vec<&str> = push.iter().map(String::as_str).collect();
                    let o = cmd::run_full(p, &refs)?;
                    out.push_str(&o.stderr);
                }
                Ok(out)
            }),
        );
    }

    /// `git push` arguments for the current branch, setting upstream if needed.
    pub fn push_args_for_head(&self) -> Option<Vec<String>> {
        let branch = self.refs.head_branch.clone()?;
        let b = self.refs.local_branch(&branch);
        match b.and_then(|b| b.upstream.clone()) {
            Some(_) => Some(vec!["push".into()]),
            None => {
                let remote = self.default_remote()?;
                Some(vec!["push".into(), "-u".into(), remote, branch])
            }
        }
    }

    pub fn default_remote(&self) -> Option<String> {
        let names = self.refs.remote_names();
        if names.iter().any(|n| n == "origin") {
            Some("origin".into())
        } else {
            names.into_iter().next()
        }
    }

    pub fn fetch_all(&mut self) {
        self.git(
            "Fetch",
            vec![
                "fetch".into(),
                "--all".into(),
                "--prune".into(),
                "--tags".into(),
            ],
        );
    }

    pub fn checkout(&mut self, target: &str) {
        self.git(
            format!("Checkout {target}"),
            vec!["checkout".into(), target.into()],
        );
    }

    /// Checks out a remote branch, creating (or reusing) a local tracking branch.
    pub fn checkout_remote(&mut self, remote_branch: &str) {
        let local = remote_branch
            .split_once('/')
            .map(|(_, b)| b)
            .unwrap_or(remote_branch);
        if self.refs.local_branch(local).is_some() {
            self.checkout(local);
        } else {
            self.git(
                format!("Checkout {remote_branch}"),
                vec![
                    "checkout".into(),
                    "-b".into(),
                    local.into(),
                    "--track".into(),
                    remote_branch.into(),
                ],
            );
        }
    }

    /// Asks for a destination and writes `path` as it is in commit `id` there.
    pub fn save_file_as(&mut self, id: String, path: String) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let Some(dest) = rfd::FileDialog::new()
            .set_title(format!("Save {name} from {}", git::short(&id)))
            .set_file_name(&name)
            .save_file()
        else {
            return;
        };
        self.run_task(
            format!("Saved {name}"),
            OpKind::Notify,
            Box::new(move |repo| {
                let (bytes, pointer) =
                    git::lfs::resolve(repo, &path, git::file_at(repo, &id, &path)?);
                std::fs::write(&dest, &bytes)
                    .map_err(|e| format!("Could not write {}: {e}", dest.display()))?;
                let size = crate::format::human_size(bytes.len() as u64);
                let note = if pointer {
                    " — LFS pointer only; the LFS object could not be fetched"
                } else {
                    ""
                };
                Ok(format!(
                    "{} ({size}) from {}{note}",
                    dest.display(),
                    git::short(&id)
                ))
            }),
        );
    }

    pub fn head_message(&self) -> Option<String> {
        git::head_message(&self.path)
    }

    pub fn open_in_file_manager(&self, sub: Option<&str>) {
        let target = sub
            .map(|s| self.path.join(s))
            .unwrap_or_else(|| self.path.clone());
        crate::platform::reveal(&target);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchAction {
    Stage,
    Unstage,
    Discard,
}

/// Keeps command output shown in the UI to a reasonable size.
fn truncate_output(s: &str) -> String {
    const MAX: usize = 20_000;
    match s.char_indices().nth(MAX) {
        Some((i, _)) => format!("{}\n… (output truncated)", &s[..i]),
        None => s.to_owned(),
    }
}

fn same_status(a: &Status, b: &Status) -> bool {
    a.staged == b.staged && a.unstaged == b.unstaged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangeKind;

    fn tab_with_files(names: &[&str]) -> RepoTab {
        let mut tab = RepoTab::open(std::env::temp_dir(), egui::Context::default(), 10);
        tab.status.unstaged = names
            .iter()
            .map(|n| FileChange {
                path: n.to_string(),
                old_path: None,
                kind: ChangeKind::Modified,
            })
            .collect();
        tab.change_order[0] = names.iter().map(|n| n.to_string()).collect();
        tab
    }

    fn selected(tab: &RepoTab) -> Vec<String> {
        tab.selected_files(false)
            .into_iter()
            .map(|f| f.path)
            .collect()
    }

    #[test]
    fn click_toggle_and_range_selection() {
        let mut tab = tab_with_files(&["a", "b", "c", "d"]);
        let none = egui::Modifiers::NONE;
        let cmd = egui::Modifiers::COMMAND;
        let shift = egui::Modifiers::SHIFT;

        tab.click_change(false, "b".into(), none);
        assert_eq!(selected(&tab), ["b"]);
        tab.click_change(false, "d".into(), shift);
        assert_eq!(selected(&tab), ["b", "c", "d"]);
        tab.click_change(false, "c".into(), cmd);
        assert_eq!(selected(&tab), ["b", "d"]);
        tab.click_change(false, "a".into(), cmd);
        assert_eq!(selected(&tab), ["a", "b", "d"]);
        // A plain click resets to a single file.
        tab.click_change(false, "c".into(), none);
        assert_eq!(selected(&tab), ["c"]);
        // Nothing is selected in the other list.
        assert!(tab.selected_files(true).is_empty());

        tab.select_all_changes(false);
        assert_eq!(selected(&tab), ["a", "b", "c", "d"]);
        tab.move_change_selection(-1, false);
        assert_eq!(selected(&tab), ["b"]);
    }
}
