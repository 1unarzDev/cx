//! Directed access policy. Discovery and reverse packet replies never grant SSH access.
use anyhow::{bail, ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Mesh,
    Downstream,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub target: String,
    pub role: Role,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// Explicit local approval, independent of reachability evidence.
    pub allowed: bool,
    /// Last authenticated SSH check from `from` to `to`.
    pub authenticated_at: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Graph {
    pub revision: u64,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Viewer authentication is independent of an address being reachable directly.
    pub grants: Vec<Edge>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    NotGranted,
    NotChecked,
    Stale,
    Authenticated,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Connection {
    pub revision: u64,
    pub target: String,
    pub jumps: Vec<String>,
    pub forward: Direction,
    pub reverse: Direction,
    pub bidirectional: bool,
}
fn identity(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && !s.starts_with('-')
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"@._-:[]".contains(&c))
}
impl Graph {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.nodes.len() <= 128 && self.edges.len() <= 1024 && self.grants.len() <= 1024,
            "access graph exceeds bounds"
        );
        let mut ids = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for n in &self.nodes {
            ensure!(
                identity(&n.id)
                    && identity(&n.target)
                    && ids.insert(&n.id)
                    && targets.insert(&n.target),
                "invalid or duplicate access identity"
            );
        }
        let mut pairs = BTreeSet::new();
        for (grant, e) in self
            .edges
            .iter()
            .map(|e| (false, e))
            .chain(self.grants.iter().map(|e| (true, e)))
        {
            ensure!(
                e.from != e.to
                    && ids.contains(&e.from)
                    && ids.contains(&e.to)
                    && pairs.insert((grant, &e.from, &e.to)),
                "invalid or duplicate directed edge"
            );
            // A downstream robot may relay onward to another downstream device,
            // but cannot acquire mesh SSH rights through this policy.
            let from = self.nodes.iter().find(|n| n.id == e.from).unwrap();
            let to = self.nodes.iter().find(|n| n.id == e.to).unwrap();
            ensure!(
                !(e.allowed && from.role == Role::Downstream && to.role == Role::Mesh),
                "downstream-to-mesh SSH grants are forbidden"
            );
        }
        Ok(())
    }
    fn evidence(&self, from: &str, to: &str, now: u64) -> Direction {
        let Some(edge) = self
            .grants
            .iter()
            .find(|e| e.from == from && e.to == to && e.allowed)
        else {
            return Direction::NotGranted;
        };
        match edge.authenticated_at {
            None => Direction::NotChecked,
            Some(at) if at > now || now - at > 90 => Direction::Stale,
            Some(_) => Direction::Authenticated,
        }
    }
    pub fn connection(&self, viewer: &str, destination: &str, now: u64) -> Result<Connection> {
        self.validate()?;
        let nodes: BTreeMap<_, _> = self.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        ensure!(
            viewer != destination && nodes.contains_key(viewer) && nodes.contains_key(destination),
            "unknown or self access endpoint"
        );
        ensure!(
            self.grants
                .iter()
                .any(|g| g.from == viewer && g.to == destination && g.allowed),
            "viewer has no approved destination SSH grant"
        );
        let mut queue = VecDeque::from([vec![viewer.to_owned()]]);
        let mut visited = BTreeSet::from([viewer.to_owned()]);
        while let Some(path) = queue.pop_front() {
            let last = path.last().unwrap();
            if last == destination {
                let jumps = path[1..path.len() - 1]
                    .iter()
                    .map(|id| nodes[id.as_str()].target.clone())
                    .collect();
                // Transit-host authentication doesn't establish that this viewer
                // possesses credentials for the final destination.
                let forward = self.evidence(viewer, destination, now);
                let reverse = self.evidence(destination, viewer, now);
                let bidirectional =
                    forward == Direction::Authenticated && reverse == Direction::Authenticated;
                return Ok(Connection {
                    revision: self.revision,
                    target: nodes[destination].target.clone(),
                    jumps,
                    forward,
                    reverse,
                    bidirectional,
                });
            }
            if path.len() >= 6 {
                continue;
            } // At most four ProxyJump hosts.
            let mut next: Vec<_> = self
                .edges
                .iter()
                .filter(|e| e.allowed && e.from == *last)
                .map(|e| e.to.clone())
                .collect();
            next.sort();
            for id in next {
                // ProxyJump authenticates every hop as the original viewer.
                // A gateway's own credentials cannot grant the viewer transit.
                if !self
                    .grants
                    .iter()
                    .any(|g| g.allowed && g.from == viewer && g.to == id)
                {
                    continue;
                }
                if visited.insert(id.clone()) {
                    let mut route = path.clone();
                    route.push(id);
                    queue.push_back(route);
                }
            }
        }
        bail!("no approved access path within four SSH jumps")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph() -> Graph {
        Graph {
            revision: 1,
            nodes: [
                ("innovation", Role::Mesh),
                ("tranquility", Role::Mesh),
                ("blastoise", Role::Downstream),
                ("sensor", Role::Downstream),
            ]
            .into_iter()
            .map(|(id, role)| Node {
                id: id.into(),
                target: format!("robot@{id}"),
                role,
            })
            .collect(),
            edges: [
                ("innovation", "tranquility"),
                ("tranquility", "blastoise"),
                ("blastoise", "sensor"),
            ]
            .into_iter()
            .map(|(from, to)| Edge {
                from: from.into(),
                to: to.into(),
                allowed: true,
                authenticated_at: Some(100),
            })
            .collect(),
            grants: [
                ("innovation", "sensor"),
                ("innovation", "tranquility"),
                ("innovation", "blastoise"),
            ]
            .into_iter()
            .map(|(from, to)| Edge {
                from: from.into(),
                to: to.into(),
                allowed: true,
                authenticated_at: None,
            })
            .collect(),
        }
    }
    #[test]
    fn recursive_access_does_not_imply_reverse_or_viewer_authentication() {
        let g = graph();
        let r = g.connection("innovation", "sensor", 100).unwrap();
        assert_eq!(r.jumps, ["robot@tranquility", "robot@blastoise"]);
        assert_eq!(r.forward, Direction::NotChecked);
        assert_eq!(r.reverse, Direction::NotGranted);
        assert!(!r.bidirectional);
        assert!(g.connection("sensor", "innovation", 100).is_err());
    }
    #[test]
    fn evidence_expiry_reverse_policy_and_cycles() {
        let mut g = graph();
        g.edges.push(Edge {
            from: "tranquility".into(),
            to: "innovation".into(),
            allowed: true,
            authenticated_at: Some(100),
        });
        g.grants[1].authenticated_at = Some(100);
        g.grants.push(Edge {
            from: "tranquility".into(),
            to: "innovation".into(),
            allowed: true,
            authenticated_at: Some(100),
        });
        assert!(
            g.connection("innovation", "tranquility", 100)
                .unwrap()
                .bidirectional
        );
        assert!(
            !g.connection("innovation", "tranquility", 191)
                .unwrap()
                .bidirectional
        );
        assert_eq!(
            g.connection("innovation", "tranquility", 99)
                .unwrap()
                .forward,
            Direction::Stale
        );
        g.edges.push(Edge {
            from: "blastoise".into(),
            to: "innovation".into(),
            allowed: true,
            authenticated_at: None,
        });
        assert!(g.validate().is_err());
    }
    #[test]
    fn denied_edge_cannot_route_and_commands_cannot_be_nodes() {
        let mut g = graph();
        g.edges[1].allowed = false;
        assert!(g.connection("innovation", "sensor", 100).is_err());
        g.nodes[0].target = "robot@host;echo bad".into();
        assert!(g.validate().is_err());
    }
    #[test]
    fn every_jump_needs_original_viewer_authorization() {
        let mut g = graph();
        g.grants.retain(|g| g.to != "blastoise");
        assert!(g.connection("innovation", "sensor", 100).is_err());
    }
}
