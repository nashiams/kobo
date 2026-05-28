//! Stage: Cluster detection via union-find partitioning.
//!
//! Partitions the constraint graph into deterministic connected components.
//! Runs containment prechecks before solving: boundary → K0090, oversized → K0081.

use std::collections::BTreeMap;

use kobo_ir::KirNodeId;

use crate::constraint_extract::{ConstraintNode, ExtractionResult, ProvenancedEdge};
use crate::solver::{BoundaryReport, ClusterReport, ConstraintEdge, SolveOutcome};

/// Unique identifier for a cluster.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ClusterId(pub u32);

/// A connected component of constraint nodes.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub id: ClusterId,
    pub nodes: Vec<ConstraintNode>,
    pub edges: Vec<ProvenancedEdge>,
    pub raw_edges: Vec<ConstraintEdge>,
    pub size: usize,
}

/// Result of containment precheck on a cluster.
pub enum ContainmentResult {
    /// Cluster is within limits — proceed to solve.
    Proceed(Cluster),
    /// Cluster contains a boundary marker — K0090.
    BoundaryStop(SolveOutcome),
    /// Cluster exceeds size limit — K0081.
    TooLarge(SolveOutcome),
}

/// Union-Find data structure for connected component detection.
struct UnionFind {
    parent: BTreeMap<KirNodeId, KirNodeId>,
    rank: BTreeMap<KirNodeId, u32>,
}

impl UnionFind {
    fn new(nodes: &[KirNodeId]) -> Self {
        let mut parent = BTreeMap::new();
        let mut rank = BTreeMap::new();
        for &node in nodes {
            parent.insert(node, node);
            rank.insert(node, 0);
        }
        Self { parent, rank }
    }

    fn find(&mut self, x: KirNodeId) -> KirNodeId {
        let p = self.parent[&x];
        if p != x {
            let root = self.find(p);
            self.parent.insert(x, root);
            root
        } else {
            x
        }
    }

    fn union(&mut self, a: KirNodeId, b: KirNodeId) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let rank_a = self.rank[&ra];
        let rank_b = self.rank[&rb];
        if rank_a < rank_b {
            self.parent.insert(ra, rb);
        } else if rank_a > rank_b {
            self.parent.insert(rb, ra);
        } else {
            self.parent.insert(rb, ra);
            self.rank.insert(ra, rank_a + 1);
        }
    }
}

/// Partition the extraction result into deterministic clusters.
///
/// Cluster IDs are stable: clusters are sorted by minimum node key.
pub fn extract_clusters(extraction: &ExtractionResult) -> Vec<Cluster> {
    if extraction.nodes.is_empty() {
        return Vec::new();
    }

    let node_ids: Vec<KirNodeId> = extraction.nodes.iter().map(|n| n.id).collect();
    let mut uf = UnionFind::new(&node_ids);

    // Union endpoints of every constraint edge.
    for edge in &extraction.edges {
        uf.union(edge.edge.source, edge.edge.target);
    }

    // Group nodes by their root.
    let mut groups: BTreeMap<KirNodeId, Vec<KirNodeId>> = BTreeMap::new();
    for &id in &node_ids {
        let root = uf.find(id);
        groups.entry(root).or_default().push(id);
    }

    // Build clusters sorted by minimum node key (deterministic).
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut sorted_groups: Vec<(KirNodeId, Vec<KirNodeId>)> = groups.into_iter().collect();
    sorted_groups.sort_by_key(|(_, members)| members.iter().copied().min());

    let node_map: BTreeMap<KirNodeId, &ConstraintNode> =
        extraction.nodes.iter().map(|n| (n.id, n)).collect();

    for (cluster_idx, (_root, member_ids)) in sorted_groups.into_iter().enumerate() {
        let member_set: std::collections::BTreeSet<KirNodeId> =
            member_ids.iter().copied().collect();

        let cluster_nodes: Vec<ConstraintNode> = member_ids
            .iter()
            .filter_map(|id| node_map.get(id).map(|n| (*n).clone()))
            .collect();

        let cluster_edges: Vec<ProvenancedEdge> = extraction
            .edges
            .iter()
            .filter(|e| member_set.contains(&e.edge.source) && member_set.contains(&e.edge.target))
            .cloned()
            .collect();

        let raw_edges: Vec<ConstraintEdge> = cluster_edges.iter().map(|e| e.edge.clone()).collect();
        let size = cluster_nodes.len();

        clusters.push(Cluster {
            id: ClusterId(cluster_idx as u32),
            nodes: cluster_nodes,
            edges: cluster_edges,
            raw_edges,
            size,
        });
    }

    clusters
}

/// Run containment precheck on a cluster.
///
/// K0090 (boundary) always wins over K0081 (too large).
pub fn containment_precheck(cluster: Cluster, cluster_limit: usize) -> ContainmentResult {
    // Check boundary first — K0090 has highest precedence.
    for node in &cluster.nodes {
        if node.is_boundary {
            return ContainmentResult::BoundaryStop(SolveOutcome::BoundaryStop(BoundaryReport {
                crossing_node: node.id,
                external_crate: "unknown".to_owned(),
                external_function: "unknown".to_owned(),
            }));
        }
    }

    // Check size limit — K0081.
    if cluster.size > cluster_limit {
        return ContainmentResult::TooLarge(SolveOutcome::ClusterTooLarge(ClusterReport {
            cluster_size: cluster.size,
            limit: cluster_limit,
            member_nodes: cluster.nodes.iter().map(|n| n.id).collect(),
        }));
    }

    ContainmentResult::Proceed(cluster)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constraint_extract::*;
    use crate::solver::{ConstraintEdge, ConstraintGraph, ConstraintKind};
    use kobo_ir::{FileId, KoboSpan};

    fn make_node(id: u32, name: &str) -> ConstraintNode {
        ConstraintNode {
            id: KirNodeId(id),
            floor: kobo_ir::OwnershipTier::PlainOwned,
            ceiling: None,
            is_boundary: false,
            binding_name: name.to_owned(),
        }
    }

    fn make_edge(src: u32, tgt: u32) -> ProvenancedEdge {
        ProvenancedEdge {
            edge: ConstraintEdge::synthetic(
                KirNodeId(src),
                KirNodeId(tgt),
                ConstraintKind::PropagateSharing,
                "test",
            ),
            provenance: ProvenanceRef {
                span: KoboSpan::new(0, 1, FileId(0)),
                fact_kind: FactKind::NeedsSharing,
                rule_name: "test",
                source_binding: None,
            },
        }
    }

    #[test]
    fn disconnected_graph_produces_multiple_clusters() {
        let extraction = ExtractionResult {
            nodes: vec![
                make_node(1, "a"),
                make_node(2, "b"),
                make_node(3, "c"),
                make_node(4, "d"),
            ],
            edges: vec![make_edge(1, 2), make_edge(3, 4)],
            graph: ConstraintGraph {
                nodes: vec![KirNodeId(1), KirNodeId(2), KirNodeId(3), KirNodeId(4)],
                edges: vec![],
            },
        };
        let clusters = extract_clusters(&extraction);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn connected_graph_produces_one_cluster() {
        let extraction = ExtractionResult {
            nodes: vec![
                make_node(1, "a"),
                make_node(2, "b"),
                make_node(3, "c"),
                make_node(4, "d"),
            ],
            edges: vec![make_edge(1, 2), make_edge(2, 3), make_edge(3, 4)],
            graph: ConstraintGraph {
                nodes: vec![KirNodeId(1), KirNodeId(2), KirNodeId(3), KirNodeId(4)],
                edges: vec![],
            },
        };
        let clusters = extract_clusters(&extraction);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].size, 4);
    }

    #[test]
    fn cluster_ordering_is_stable() {
        let extraction = ExtractionResult {
            nodes: vec![
                make_node(10, "z"),
                make_node(1, "a"),
                make_node(20, "y"),
                make_node(2, "b"),
            ],
            edges: vec![make_edge(1, 2), make_edge(10, 20)],
            graph: ConstraintGraph {
                nodes: vec![KirNodeId(10), KirNodeId(1), KirNodeId(20), KirNodeId(2)],
                edges: vec![],
            },
        };
        let c1 = extract_clusters(&extraction);
        let c2 = extract_clusters(&extraction);
        assert_eq!(c1[0].id, c2[0].id);
        assert_eq!(c1[1].id, c2[1].id);
    }

    #[test]
    fn boundary_wins_over_oversized() {
        let mut node = make_node(1, "a");
        node.is_boundary = true;
        let cluster = Cluster {
            id: ClusterId(0),
            nodes: vec![node],
            edges: Vec::new(),
            raw_edges: Vec::new(),
            size: 1,
        };
        // Even with cluster_limit=0, boundary should win.
        let result = containment_precheck(cluster, 0);
        assert!(matches!(result, ContainmentResult::BoundaryStop(_)));
    }

    #[test]
    fn oversized_non_boundary_returns_k0081() {
        let cluster = Cluster {
            id: ClusterId(0),
            nodes: vec![make_node(1, "a"), make_node(2, "b"), make_node(3, "c")],
            edges: Vec::new(),
            raw_edges: Vec::new(),
            size: 3,
        };
        let result = containment_precheck(cluster, 2);
        assert!(matches!(result, ContainmentResult::TooLarge(_)));
    }

    #[test]
    fn adding_edge_reduces_cluster_count() {
        let extraction_disconnected = ExtractionResult {
            nodes: vec![make_node(1, "a"), make_node(2, "b")],
            edges: vec![],
            graph: ConstraintGraph {
                nodes: vec![KirNodeId(1), KirNodeId(2)],
                edges: vec![],
            },
        };
        let extraction_connected = ExtractionResult {
            nodes: vec![make_node(1, "a"), make_node(2, "b")],
            edges: vec![make_edge(1, 2)],
            graph: ConstraintGraph {
                nodes: vec![KirNodeId(1), KirNodeId(2)],
                edges: vec![],
            },
        };
        assert_eq!(extract_clusters(&extraction_disconnected).len(), 2);
        assert_eq!(extract_clusters(&extraction_connected).len(), 1);
    }
}
