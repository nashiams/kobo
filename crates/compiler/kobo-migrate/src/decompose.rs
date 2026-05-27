//! Stage: Cluster decomposition with bridge variables.
//!
//! Decomposes oversized clusters into sub-clusters by identifying
//! bridge variables (articulation points) that connect sub-graphs.

use std::collections::{BTreeMap, BTreeSet};

use kobo_ir::KirNodeId;

use crate::cluster::{Cluster, ClusterId};
use crate::constraint_extract::{ConstraintNode, ProvenancedEdge};
use crate::solver::ConstraintEdge;

/// A bridge variable that connects two sub-clusters.
#[derive(Clone, Debug)]
pub struct BridgeVariable {
    pub node_id: KirNodeId,
    pub connects: (ClusterId, ClusterId),
}

/// Result of decomposition.
pub struct DecomposeResult {
    pub sub_clusters: Vec<Cluster>,
    pub bridges: Vec<BridgeVariable>,
}

/// Decompose a cluster into sub-clusters by removing articulation points.
///
/// If the cluster cannot be decomposed (no articulation points), returns
/// the original cluster as the sole sub-cluster.
pub fn decompose(cluster: &Cluster, target_size: usize) -> DecomposeResult {
    if cluster.size <= target_size {
        return DecomposeResult {
            sub_clusters: vec![cluster.clone()],
            bridges: Vec::new(),
        };
    }

    // Find articulation points.
    let articulation_points = find_articulation_points(cluster);

    if articulation_points.is_empty() {
        // No articulation points — cluster is biconnected, cannot decompose.
        return DecomposeResult {
            sub_clusters: vec![cluster.clone()],
            bridges: Vec::new(),
        };
    }

    // Remove articulation points and find connected components.
    let art_set: BTreeSet<KirNodeId> = articulation_points.iter().copied().collect();
    let remaining_nodes: Vec<KirNodeId> = cluster
        .nodes
        .iter()
        .map(|n| n.id)
        .filter(|id| !art_set.contains(id))
        .collect();

    // Build adjacency without articulation points.
    let mut adj: BTreeMap<KirNodeId, BTreeSet<KirNodeId>> = BTreeMap::new();
    for edge in &cluster.raw_edges {
        if !art_set.contains(&edge.source) && !art_set.contains(&edge.target) {
            adj.entry(edge.source).or_default().insert(edge.target);
            adj.entry(edge.target).or_default().insert(edge.source);
        }
    }

    // BFS to find components.
    let mut visited = BTreeSet::new();
    let mut components: Vec<Vec<KirNodeId>> = Vec::new();
    for &node in &remaining_nodes {
        if visited.contains(&node) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(node);
        visited.insert(node);
        while let Some(current) = queue.pop_front() {
            component.push(current);
            if let Some(neighbors) = adj.get(&current) {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor);
                        queue.push_back(neighbor);
                    }
                }
            }
        }
        components.push(component);
    }

    // Build sub-clusters with articulation points re-attached to adjacent components.
    let node_map: BTreeMap<KirNodeId, &ConstraintNode> =
        cluster.nodes.iter().map(|n| (n.id, n)).collect();

    let mut sub_clusters = Vec::new();
    let mut bridges = Vec::new();

    for (comp_idx, component) in components.iter().enumerate() {
        let comp_set: BTreeSet<KirNodeId> = component.iter().copied().collect();

        // Add articulation points that are adjacent to this component.
        let mut extended_set = comp_set.clone();
        for &art in &articulation_points {
            let adjacent = cluster.raw_edges.iter().any(|e| {
                (e.source == art && comp_set.contains(&e.target))
                    || (e.target == art && comp_set.contains(&e.source))
            });
            if adjacent {
                extended_set.insert(art);
            }
        }

        let sub_nodes: Vec<ConstraintNode> = extended_set
            .iter()
            .filter_map(|id| node_map.get(id).map(|n| (*n).clone()))
            .collect();
        let sub_edges: Vec<ProvenancedEdge> = cluster
            .edges
            .iter()
            .filter(|e| {
                extended_set.contains(&e.edge.source) && extended_set.contains(&e.edge.target)
            })
            .cloned()
            .collect();
        let raw_edges: Vec<ConstraintEdge> = sub_edges.iter().map(|e| e.edge.clone()).collect();
        let size = sub_nodes.len();

        sub_clusters.push(Cluster {
            id: ClusterId(comp_idx as u32),
            nodes: sub_nodes,
            edges: sub_edges,
            raw_edges,
            size,
        });
    }

    // Record bridges.
    for &art in &articulation_points {
        // Find which sub-clusters this articulation point appears in.
        let mut containing: Vec<ClusterId> = Vec::new();
        for sc in &sub_clusters {
            if sc.nodes.iter().any(|n| n.id == art) {
                containing.push(sc.id);
            }
        }
        if containing.len() >= 2 {
            bridges.push(BridgeVariable {
                node_id: art,
                connects: (containing[0], containing[1]),
            });
        }
    }

    DecomposeResult {
        sub_clusters,
        bridges,
    }
}

/// Find articulation points using DFS.
fn find_articulation_points(cluster: &Cluster) -> Vec<KirNodeId> {
    let nodes: Vec<KirNodeId> = cluster.nodes.iter().map(|n| n.id).collect();
    if nodes.len() <= 2 {
        return Vec::new();
    }

    let mut adj: BTreeMap<KirNodeId, BTreeSet<KirNodeId>> = BTreeMap::new();
    for edge in &cluster.raw_edges {
        adj.entry(edge.source).or_default().insert(edge.target);
        adj.entry(edge.target).or_default().insert(edge.source);
    }

    let mut disc: BTreeMap<KirNodeId, u32> = BTreeMap::new();
    let mut low: BTreeMap<KirNodeId, u32> = BTreeMap::new();
    let mut parent: BTreeMap<KirNodeId, Option<KirNodeId>> = BTreeMap::new();
    let mut ap_set: BTreeSet<KirNodeId> = BTreeSet::new();
    let mut timer: u32 = 0;

    for &start in &nodes {
        if disc.contains_key(&start) {
            continue;
        }
        // Iterative DFS.
        let mut stack: Vec<(KirNodeId, bool)> = vec![(start, false)];
        parent.insert(start, None);

        while let Some((u, returning)) = stack.last().copied() {
            if !returning {
                if disc.contains_key(&u) {
                    stack.pop();
                    continue;
                }
                disc.insert(u, timer);
                low.insert(u, timer);
                timer += 1;

                // Mark as "entering" — we'll revisit as "returning".
                if let Some(last) = stack.last_mut() {
                    last.1 = true;
                }

                let neighbors: Vec<KirNodeId> = adj
                    .get(&u)
                    .map(|s| s.iter().copied().collect())
                    .unwrap_or_default();

                for v in neighbors {
                    if !disc.contains_key(&v) {
                        parent.insert(v, Some(u));
                        stack.push((v, false));
                    } else if Some(Some(v)) != parent.get(&u).copied() {
                        // Back edge.
                        let dv = disc[&v];
                        let lu = low[&u];
                        if dv < lu {
                            low.insert(u, dv);
                        }
                    }
                }
            } else {
                stack.pop();
                let u_low = low[&u];
                let _u_disc = disc[&u];

                // Update parent's low.
                if let Some(Some(p)) = parent.get(&u) {
                    let pl = low[p];
                    if u_low < pl {
                        low.insert(*p, u_low);
                    }
                    // Check articulation condition.
                    if parent.get(p).copied().flatten().is_some() {
                        // Non-root: u_low >= disc[p] means p is AP.
                        let pd = disc[p];
                        if u_low >= pd {
                            ap_set.insert(*p);
                        }
                    }
                }

                // Root check: if root has >= 2 children in DFS tree.
                if parent.get(&u).copied().flatten().is_none() {
                    let child_count = nodes
                        .iter()
                        .filter(|&&v| parent.get(&v).copied().flatten() == Some(u))
                        .count();
                    if child_count >= 2 {
                        ap_set.insert(u);
                    }
                }
            }
        }
    }

    ap_set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint_extract::ConstraintNode;
    use crate::solver::{ConstraintEdge, ConstraintKind};
    use kobo_ir::{KirNodeId, OwnershipTier};

    fn node(id: u32) -> ConstraintNode {
        ConstraintNode {
            id: KirNodeId(id),
            floor: OwnershipTier::PlainOwned,
            ceiling: None,
            is_boundary: false,
            binding_name: format!("v{}", id),
        }
    }

    fn edge(s: u32, t: u32) -> ConstraintEdge {
        ConstraintEdge::synthetic(
            KirNodeId(s),
            KirNodeId(t),
            ConstraintKind::PropagateSharing,
            "test",
        )
    }

    #[test]
    fn small_cluster_not_decomposed() {
        let c = Cluster {
            id: ClusterId(0),
            nodes: vec![node(1), node(2)],
            edges: Vec::new(),
            raw_edges: vec![edge(1, 2)],
            size: 2,
        };
        let result = decompose(&c, 10);
        assert_eq!(result.sub_clusters.len(), 1);
        assert!(result.bridges.is_empty());
    }

    #[test]
    fn linear_chain_decomposes_at_articulation() {
        // 1-2-3-4-5: node 2,3,4 are articulation points
        let c = Cluster {
            id: ClusterId(0),
            nodes: vec![node(1), node(2), node(3), node(4), node(5)],
            edges: Vec::new(),
            raw_edges: vec![edge(1, 2), edge(2, 3), edge(3, 4), edge(4, 5)],
            size: 5,
        };
        let result = decompose(&c, 2);
        // Should produce multiple sub-clusters.
        assert!(result.sub_clusters.len() >= 2);
    }
}
