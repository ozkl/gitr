//! Lane assignment for drawing the commit graph.

use std::collections::HashMap;

use super::model::Commit;

/// A line from lane `from` in the previous row to lane `to` in this row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: u16,
    pub to: u16,
    pub color: u16,
}

#[derive(Debug, Clone, Default)]
pub struct GraphRow {
    pub lane: u16,
    pub color: u16,
    /// Edges entering this row from the row above.
    pub up: Vec<Edge>,
    /// Number of lanes in use at this row (for sizing the graph column).
    pub width: u16,
    pub is_merge: bool,
}

struct Lane {
    target: String,
    color: u16,
    /// Lane in the previous row the line starts from (when it branches off a node).
    source: Option<u16>,
}

pub fn build(commits: &[Commit]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<Lane>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    let mut next_color: u16 = 0;
    let present: HashMap<&str, ()> = commits.iter().map(|c| (c.id.as_str(), ())).collect();

    for commit in commits {
        let matching: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, l)| l.as_ref().is_some_and(|l| l.target == commit.id))
            .map(|(i, _)| i)
            .collect();

        let node_lane = match matching.first() {
            Some(&i) => i,
            None => match lanes.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    lanes.push(None);
                    lanes.len() - 1
                }
            },
        };
        let color = match matching.first() {
            Some(&i) => lanes[i].as_ref().unwrap().color,
            None => {
                next_color = next_color.wrapping_add(1);
                next_color
            }
        };

        // Edges coming in from the previous row.
        let mut up = Vec::new();
        for (i, lane) in lanes.iter().enumerate() {
            let Some(lane) = lane else { continue };
            let from = lane.source.unwrap_or(i as u16);
            let to = if lane.target == commit.id { node_lane } else { i };
            up.push(Edge { from, to: to as u16, color: lane.color });
        }

        // Lanes that converged into this commit are freed.
        for &i in &matching {
            lanes[i] = None;
        }
        for lane in lanes.iter_mut().flatten() {
            lane.source = None;
        }

        // Continue with the parents.
        let parents: Vec<&String> = commit
            .parents
            .iter()
            .filter(|p| present.contains_key(p.as_str()))
            .collect();
        for (pi, parent) in parents.iter().enumerate() {
            if pi == 0 {
                lanes[node_lane] = Some(Lane { target: (*parent).clone(), color, source: None });
                continue;
            }
            // Every extra parent gets its own lane; duplicate lanes heading to the same
            // commit converge when that commit is reached.
            next_color = next_color.wrapping_add(1);
            let new_lane = Lane {
                target: (*parent).clone(),
                color: next_color,
                source: Some(node_lane as u16),
            };
            match lanes.iter().position(Option::is_none) {
                Some(i) => lanes[i] = Some(new_lane),
                None => lanes.push(Some(new_lane)),
            }
        }

        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }

        let width = up
            .iter()
            .map(|e| e.from.max(e.to) + 1)
            .chain(std::iter::once(node_lane as u16 + 1))
            .chain(std::iter::once(lanes.len() as u16))
            .max()
            .unwrap_or(1);

        rows.push(GraphRow {
            lane: node_lane as u16,
            color,
            up,
            width,
            is_merge: commit.parents.len() > 1,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            subject: String::new(),
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = build(&[c("c", &["b"]), c("b", &["a"]), c("a", &[])]);
        assert!(rows.iter().all(|r| r.lane == 0));
        assert_eq!(rows[1].up, vec![Edge { from: 0, to: 0, color: rows[0].color }]);
    }

    #[test]
    fn merge_opens_and_closes_second_lane() {
        // m merges b into a-line: m -> a1, m -> b ; b -> a0 ; a1 -> a0
        let rows = build(&[
            c("m", &["a1", "b"]),
            c("b", &["a0"]),
            c("a1", &["a0"]),
            c("a0", &[]),
        ]);
        assert_eq!(rows[0].lane, 0);
        assert_eq!(rows[1].lane, 1);
        // Edge from merge node (lane 0) into the second lane.
        assert!(rows[1].up.contains(&Edge { from: 0, to: 1, color: rows[1].color }));
        assert_eq!(rows[2].lane, 0);
        assert_eq!(rows[3].lane, 0);
        // Both lanes converge into a0.
        assert_eq!(rows[3].up.len(), 2);
        assert!(rows[3].up.iter().all(|e| e.to == 0));
    }
}
