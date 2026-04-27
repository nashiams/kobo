//! Stress tests for the Kobo ownership solver.
//!
//! These tests exercise edge-case cluster shapes: large graphs, cycles,
//! empty inputs, self-loops, disconnected components, and complete graphs.

use kobo_ir::{KirNodeId, OwnershipTier};
use kobo_migrate::cluster::{Cluster, ClusterId};
use kobo_migrate::constraint_extract::ConstraintNode;
use kobo_migrate::lattice_solve::{lattice_solve, LatticeOutcome};
use kobo_migrate::{ConstraintEdge, ConstraintKind};

// ─────────────────── Helpers ───────────────────

fn mk_node(id: u32, floor: OwnershipTier) -> ConstraintNode {
    ConstraintNode {
        id: KirNodeId(id),
        floor,
        ceiling: None,
        is_boundary: false,
        binding_name: format!("v{}", id),
    }
}

fn mk_node_bounded(id: u32, floor: OwnershipTier, ceiling: OwnershipTier) -> ConstraintNode {
    ConstraintNode {
        id: KirNodeId(id),
        floor,
        ceiling: Some(ceiling),
        is_boundary: false,
        binding_name: format!("v{}", id),
    }
}

fn mk_edge(src: u32, tgt: u32, kind: ConstraintKind) -> ConstraintEdge {
    ConstraintEdge::synthetic(KirNodeId(src), KirNodeId(tgt), kind, "stress-test")
}

fn mk_cluster(nodes: Vec<ConstraintNode>, edges: Vec<ConstraintEdge>) -> Cluster {
    let size = nodes.len();
    Cluster {
        id: ClusterId(0),
        nodes,
        edges: Vec::new(),
        raw_edges: edges,
        size,
    }
}

// ═══════════════════════════════════════════════════
// 1. Large clusters terminate within budget
// ═══════════════════════════════════════════════════

#[test]
fn large_cluster_100_nodes_terminates() {
    let n = 100u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Chain with one high-floor node at the start.
    nodes.push(mk_node(0, OwnershipTier::ArcShared));
    for i in 1..n {
        nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        edges.push(mk_edge(i - 1, i, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    let outcome = lattice_solve(&cluster);

    match outcome {
        LatticeOutcome::Solved(map) => {
            // All nodes should converge to ArcShared.
            for i in 0..n {
                let tier = map.get(KirNodeId(i)).unwrap();
                assert_eq!(
                    tier,
                    OwnershipTier::ArcShared,
                    "Node v{} should be ArcShared, got {:?}",
                    i,
                    tier
                );
            }
        }
        other => panic!("100-node chain should solve, got {:?}", other),
    }
}

#[test]
fn large_cluster_500_nodes_terminates() {
    let n = 500u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    nodes.push(mk_node(0, OwnershipTier::RcShared));
    for i in 1..n {
        nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        edges.push(mk_edge(i - 1, i, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    let outcome = lattice_solve(&cluster);

    assert!(
        matches!(outcome, LatticeOutcome::Solved(_)),
        "500-node chain should solve"
    );
}

#[test]
fn large_cluster_1000_nodes_terminates() {
    let n = 1000u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    nodes.push(mk_node(0, OwnershipTier::RcShared));
    for i in 1..n {
        nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        edges.push(mk_edge(i - 1, i, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    let outcome = lattice_solve(&cluster);

    assert!(
        matches!(outcome, LatticeOutcome::Solved(_)),
        "1000-node chain should solve"
    );
}

// ═══════════════════════════════════════════════════
// 2. Cyclic constraint graphs: ring of 50 nodes
// ═══════════════════════════════════════════════════

#[test]
fn cyclic_ring_50_nodes_converges() {
    let n = 50u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // One node with a high floor; all others at PlainOwned.
    // In a ring, PropagateSharing should raise everyone to the max.
    for i in 0..n {
        if i == 0 {
            nodes.push(mk_node(i, OwnershipTier::ArcShared));
        } else {
            nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        }
        edges.push(mk_edge(i, (i + 1) % n, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            // All should converge to ArcShared (the max floor).
            for i in 0..n {
                assert_eq!(
                    map.get(KirNodeId(i)).unwrap(),
                    OwnershipTier::ArcShared,
                    "Node v{} in ring should converge to ArcShared",
                    i
                );
            }
        }
        other => panic!("Ring of 50 should converge, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════
// 3. Conflicting floors/ceilings in chain
// ═══════════════════════════════════════════════════

#[test]
fn conflicting_floor_ceiling_chain() {
    // Node 0: floor=ArcMutShared. Node 9: ceiling=RcShared.
    // Chain 0→1→...→9 with PropagateSharing.
    // ArcMutShared propagates to node 9 which has ceiling=RcShared → Conflict.
    let n = 10u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for i in 0..n {
        if i == 0 {
            nodes.push(mk_node(i, OwnershipTier::ArcMutShared));
        } else if i == n - 1 {
            nodes.push(mk_node_bounded(
                i,
                OwnershipTier::PlainOwned,
                OwnershipTier::RcShared,
            ));
        } else {
            nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        }
        if i > 0 {
            edges.push(mk_edge(i - 1, i, ConstraintKind::PropagateSharing));
        }
    }

    let cluster = mk_cluster(nodes, edges);
    match lattice_solve(&cluster) {
        LatticeOutcome::Conflict { .. } => {
            // Expected: ArcMutShared propagates through chain, hits RcShared ceiling.
        }
        LatticeOutcome::Solved(_) => {
            panic!("Should conflict: ArcMutShared cannot fit in RcShared ceiling");
        }
        LatticeOutcome::IterationBudgetExceeded { .. } => {
            panic!("Should not exceed budget on 10-node chain");
        }
    }
}

// ═══════════════════════════════════════════════════
// 4. Budget edge case
// ═══════════════════════════════════════════════════

#[test]
fn budget_edge_case_respected() {
    // A trivially solvable cluster. Verify that the budget formula
    // (size * 64, min 1024) does not interfere with normal solving.
    let nodes = vec![
        mk_node(0, OwnershipTier::RcShared),
        mk_node(1, OwnershipTier::PlainOwned),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(map.get(KirNodeId(1)).unwrap(), OwnershipTier::RcShared);
        }
        other => panic!("Trivial 2-node cluster should solve, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════
// 5. Empty cluster
// ═══════════════════════════════════════════════════

#[test]
fn empty_cluster_solves_with_empty_map() {
    let cluster = mk_cluster(vec![], vec![]);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(map.len(), 0, "Empty cluster should produce empty solution");
        }
        other => panic!("Empty cluster should solve, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════
// 6. Self-loops: no infinite loop
// ═══════════════════════════════════════════════════

#[test]
fn self_loop_does_not_hang() {
    // Node with an edge to itself.
    let nodes = vec![mk_node(0, OwnershipTier::RcShared)];
    let edges = vec![mk_edge(0, 0, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(map.get(KirNodeId(0)).unwrap(), OwnershipTier::RcShared);
        }
        LatticeOutcome::IterationBudgetExceeded { .. } => {
            panic!("Self-loop should not exhaust budget");
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("Self-loop with consistent floor should not conflict");
        }
    }
}

// ═══════════════════════════════════════════════════
// 7. Disconnected components within one cluster
// ═══════════════════════════════════════════════════

#[test]
fn disconnected_components_both_solved() {
    // Two independent subgraphs: {v0, v1} and {v2, v3}.
    let nodes = vec![
        mk_node(0, OwnershipTier::RcShared),
        mk_node(1, OwnershipTier::PlainOwned),
        mk_node(2, OwnershipTier::ArcShared),
        mk_node(3, OwnershipTier::PlainOwned),
    ];
    let edges = vec![
        mk_edge(0, 1, ConstraintKind::PropagateSharing),
        mk_edge(2, 3, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            // Component 1: both should be RcShared.
            assert_eq!(map.get(KirNodeId(0)).unwrap(), OwnershipTier::RcShared);
            assert_eq!(map.get(KirNodeId(1)).unwrap(), OwnershipTier::RcShared);
            // Component 2: both should be ArcShared.
            assert_eq!(map.get(KirNodeId(2)).unwrap(), OwnershipTier::ArcShared);
            assert_eq!(map.get(KirNodeId(3)).unwrap(), OwnershipTier::ArcShared);
        }
        other => panic!("Disconnected components should solve, got {:?}", other),
    }
}

// ═══════════════════════════════════════════════════
// 8. Complete graph: all converge to highest floor
// ═══════════════════════════════════════════════════

#[test]
fn complete_graph_20_nodes_converges_to_max_floor() {
    let n = 20u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // All nodes at PlainOwned except node 0 at ArcMutShared.
    for i in 0..n {
        if i == 0 {
            nodes.push(mk_node(i, OwnershipTier::ArcMutShared));
        } else {
            nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        }
    }

    // All-to-all PropagateSharing edges.
    for i in 0..n {
        for j in 0..n {
            if i != j {
                edges.push(mk_edge(i, j, ConstraintKind::PropagateSharing));
            }
        }
    }

    let cluster = mk_cluster(nodes, edges);
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            for i in 0..n {
                assert_eq!(
                    map.get(KirNodeId(i)).unwrap(),
                    OwnershipTier::ArcMutShared,
                    "Node v{} in complete graph should converge to ArcMutShared",
                    i
                );
            }
        }
        other => panic!(
            "Complete graph with no ceilings should solve, got {:?}",
            other
        ),
    }
}
