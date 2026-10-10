//! Workspaces: named sets of repositories, each with its own open tabs.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_NAME: &str = "Home";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub name: String,
    /// Repositories listed in this workspace.
    pub repos: Vec<PathBuf>,
    /// Repositories open as tabs, in tab order.
    pub open_tabs: Vec<PathBuf>,
    pub active_tab: Option<usize>,
}

/// One row of the Configure dialog: the workspace it came from (`None` for a new one) and
/// its (possibly edited) name.
pub type Edit = (Option<usize>, String);

/// The saved workspaces, or a single "Home" workspace built from the state of versions that
/// had no workspaces. Returns the list and the index of the current one.
pub fn load(
    saved: Vec<Workspace>,
    current: usize,
    legacy_repos: Vec<PathBuf>,
    legacy_tabs: Vec<PathBuf>,
    legacy_active: Option<usize>,
) -> (Vec<Workspace>, usize) {
    if saved.is_empty() {
        let home = Workspace {
            name: DEFAULT_NAME.to_owned(),
            repos: legacy_repos,
            open_tabs: legacy_tabs,
            active_tab: legacy_active,
        };
        return (vec![home], 0);
    }
    let current = current.min(saved.len() - 1);
    (saved, current)
}

/// Why a set of names cannot be saved, if it cannot.
pub fn validate(edits: &[Edit]) -> Option<&'static str> {
    if edits.is_empty() {
        return Some("At least one workspace is needed.");
    }
    let names: Vec<String> = edits.iter().map(|(_, n)| n.trim().to_lowercase()).collect();
    if names.iter().any(String::is_empty) {
        return Some("Every workspace needs a name.");
    }
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != names.len() {
        return Some("Workspace names must be different.");
    }
    None
}

/// Applies the Configure dialog's result: renames, removals, additions and the new order.
/// Returns the new list and the index of the workspace that should be current: the same one
/// as before if it still exists, otherwise the first.
pub fn apply_edits(old: &[Workspace], current: usize, edits: &[Edit]) -> (Vec<Workspace>, usize) {
    let mut new_current = 0;
    let workspaces = edits
        .iter()
        .enumerate()
        .map(|(i, (source, name))| {
            let mut ws = source.and_then(|s| old.get(s)).cloned().unwrap_or_default();
            ws.name = name.trim().to_owned();
            if *source == Some(current) {
                new_current = i;
            }
            ws
        })
        .collect();
    (workspaces, new_current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(name: &str, repos: &[&str]) -> Workspace {
        Workspace {
            name: name.to_owned(),
            repos: repos.iter().map(PathBuf::from).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn legacy_state_becomes_the_home_workspace() {
        let (list, current) = load(
            Vec::new(),
            3,
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            vec![PathBuf::from("/b")],
            Some(0),
        );
        assert_eq!(current, 0);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Home");
        assert_eq!(list[0].repos.len(), 2);
        assert_eq!(list[0].open_tabs, [PathBuf::from("/b")]);
        // Saved workspaces win over legacy fields; an out-of-range index is clamped.
        let (list, current) = load(
            vec![ws("A", &[]), ws("B", &[])],
            9,
            vec![PathBuf::from("/x")],
            Vec::new(),
            None,
        );
        assert_eq!((list.len(), current), (2, 1));
        assert!(list[0].repos.is_empty());
    }

    #[test]
    fn edits_rename_remove_add_and_reorder() {
        let old = vec![
            ws("Home", &["/h"]),
            ws("Work", &["/w1", "/w2"]),
            ws("Old", &["/o"]),
        ];
        // Reorder Work first (renamed), keep Home, drop Old, add a new one. Current was Work.
        let edits = vec![
            (Some(1), " Job ".to_owned()),
            (Some(0), "Home".to_owned()),
            (None, "Play".to_owned()),
        ];
        let (new, current) = apply_edits(&old, 1, &edits);
        assert_eq!(
            new.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
            ["Job", "Home", "Play"]
        );
        assert_eq!(new[0].repos.len(), 2, "renaming keeps the repositories");
        assert!(new[2].repos.is_empty());
        assert_eq!(
            current, 0,
            "the current workspace is followed to its new position"
        );
        // Removing the current workspace falls back to the first one.
        let (_, current) = apply_edits(&old, 2, &edits);
        assert_eq!(current, 0);
    }

    #[test]
    fn validates_names() {
        let e = |names: &[&str]| {
            names
                .iter()
                .map(|n| (None, n.to_string()))
                .collect::<Vec<Edit>>()
        };
        assert!(validate(&e(&["Home", "Work"])).is_none());
        assert!(validate(&e(&[])).is_some());
        assert!(validate(&e(&["Home", "  "])).is_some());
        assert!(validate(&e(&["Home", "home "])).is_some());
    }
}
