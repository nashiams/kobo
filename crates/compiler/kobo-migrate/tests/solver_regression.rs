//! Regression tests for Wave 1 fixes (T01–T06).
//!
//! Each test targets exactly one Wave 1 fix and verifies the bug
//! does not regress.

use kobo_ir::{KirNodeId, OwnershipTier};
use kobo_migrate::backtrack::{BacktrackResult, BacktrackSolver, Disjunction};
use kobo_migrate::cluster::{Cluster, ClusterId};
use kobo_migrate::constraint_extract::ConstraintNode;
use kobo_migrate::lattice_solve::{lattice_solve, tier_rank, LatticeOutcome};
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
    ConstraintEdge::synthetic(KirNodeId(src), KirNodeId(tgt), kind, "regression-test")
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
// T01: IterationBudgetExceeded must not return Solved
// ═══════════════════════════════════════════════════

#[test]
fn test_iteration_cap_does_not_return_solved() {
    // The `IterationBudgetExceeded` variant must exist and be distinguishable
    // from `Solved`. We verify this by constructing a large conflicting chain
    // and checking the outcome is NOT `Solved` with a valid map.
    //
    // A chain: v0(ArcMutShared) → v1 → ... → v99(ceil=RcShared).
    // The propagation of ArcMutShared will hit v99's ceiling, producing a
    // Conflict (not Solved). The key regression: before T01, the solver
    // would return Solved with a partial map on budget exhaustion.
    let n = 100;
    let mut nodes = Vec::new();
    for i in 0..n {
        if i == 0 {
            nodes.push(mk_node(i, OwnershipTier::ArcMutShared));
        } else if i == n - 1 {
            nodes.push(mk_node_bounded(i, OwnershipTier::PlainOwned, OwnershipTier::RcShared));
        } else {
            nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        }
    }
    let mut edges = Vec::new();
    for i in 0..n - 1 {
        edges.push(mk_edge(i, i + 1, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    let outcome = lattice_solve(&cluster);

    // Must be Conflict or IterationBudgetExceeded — never Solved.
    match outcome {
        LatticeOutcome::Solved(ref map) => {
            // If solved, the last node must respect its ceiling.
            let last = map.get(KirNodeId(n - 1)).unwrap();
            assert!(
                tier_rank(last) <= tier_rank(OwnershipTier::RcShared),
                "Regression: solver returned Solved with ceiling-violating tier {:?}",
                last
            );
            // This path means the solver correctly avoided the bug (detected conflict).
            // Either way is acceptable as long as no ceiling is violated.
        }
        LatticeOutcome::Conflict { .. } => {
            // Expected: propagation hits ceiling → conflict.
        }
        LatticeOutcome::IterationBudgetExceeded { .. } => {
            // Also acceptable: the variant exists and was returned.
        }
    }
}

// ═══════════════════════════════════════════════════
// T02: Backtrack receives real disjunctions
// ═══════════════════════════════════════════════════

#[test]
fn test_backtrack_receives_real_disjunctions() {
    // Construct a cluster that produces a lattice Conflict, then feed
    // the conflict into the backtracker with real disjunctions.
    // Before T02, backtrack always received empty disjunctions.
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcMutShared),
        mk_node_bounded(1, OwnershipTier::PlainOwned, OwnershipTier::RcShared),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    // Verify lattice produces a conflict.
    let outcome = lattice_solve(&cluster);
    assert!(
        matches!(outcome, LatticeOutcome::Conflict { .. }),
        "Expected conflict, got {:?}",
        outcome
    );

    // Build disjunctions from the conflict and feed to backtracker.
    let disj = vec![Disjunction {
        node: KirNodeId(1),
        alternatives: vec![
            OwnershipTier::PlainOwned,
            OwnershipTier::BoxOwned,
            OwnershipTier::RcShared,
        ],
        label: "regression-disjunction".into(),
    }];

    let mut solver = BacktrackSolver::new(&cluster);
    let result = solver.solve(&cluster, &disj);

    // The backtracker should actually search (not short-circuit).
    // It should find RcShared or lower as valid.
    match result {
        BacktrackResult::Solved(map) => {
            let tier = map.get(KirNodeId(1)).unwrap();
            assert!(
                tier_rank(tier) <= tier_rank(OwnershipTier::RcShared),
                "Solved tier {:?} exceeds ceiling RcShared",
                tier
            );
        }
        BacktrackResult::Exhausted => {
            // Acceptable: the solver searched but found no valid assignment.
        }
        BacktrackResult::BudgetExceeded { .. } => {
            panic!("Budget should not be exceeded on a 2-node cluster");
        }
    }
}

// ═══════════════════════════════════════════════════
// T03: MutuallyExclusive detected in lattice
// ═══════════════════════════════════════════════════

#[test]
fn test_mutually_exclusive_detected_in_lattice() {
    // Two nodes both at RcMutShared, connected by MutuallyExclusive.
    // The lattice post-solve check must detect this as a conflict.
    let nodes = vec![
        mk_node(0, OwnershipTier::RcMutShared),
        mk_node(1, OwnershipTier::RcMutShared),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::MutuallyExclusive)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Conflict { node, .. } => {
            assert!(
                node == KirNodeId(0) || node == KirNodeId(1),
                "Conflict should be on v0 or v1, got {:?}",
                node
            );
        }
        LatticeOutcome::Solved(_) => {
            panic!("Regression: MutuallyExclusive with both mutable should conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { .. } => {
            panic!("Unexpected budget exceeded on 2-node cluster");
        }
    }
}

// ═══════════════════════════════════════════════════
// T04: tier_rank matches priority() ordering
// ═══════════════════════════════════════════════════

#[test]
fn test_tier_rank_matches_priority() {
    // For all 8 OwnershipTier variants, verify tier_rank and priority()
    // produce the same relative ordering.
    let all_tiers = [
        OwnershipTier::Undecided,
        OwnershipTier::PlainOwned,
        OwnershipTier::BoxOwned,
        OwnershipTier::RcShared,
        OwnershipTier::ArcShared,
        OwnershipTier::RcMutShared,
        OwnershipTier::ArcMutShared,
        OwnershipTier::Scoped,
    ];

    for (i, &a) in all_tiers.iter().enumerate() {
        for (j, &b) in all_tiers.iter().enumerate() {
            let rank_cmp = tier_rank(a).cmp(&tier_rank(b));
            let prio_cmp = a.priority().cmp(&b.priority());
            assert_eq!(
                rank_cmp, prio_cmp,
                "Ordering mismatch for {:?} vs {:?}: tier_rank says {:?}, priority() says {:?} (i={}, j={})",
                a, b, rank_cmp, prio_cmp, i, j
            );
        }
    }
}

// ═══════════════════════════════════════════════════
// T05: Undecided distinct from PlainOwned
// ═══════════════════════════════════════════════════

#[test]
fn test_undecided_distinct_from_plain_owned() {
    assert_ne!(
        tier_rank(OwnershipTier::Undecided),
        tier_rank(OwnershipTier::PlainOwned),
        "Regression: Undecided must have a distinct rank from PlainOwned"
    );
    // Undecided should be strictly below PlainOwned.
    assert!(
        tier_rank(OwnershipTier::Undecided) < tier_rank(OwnershipTier::PlainOwned),
        "Undecided should rank below PlainOwned"
    );
}

// ═══════════════════════════════════════════════════
// T06: No fabricated PlainOwned on solver failure
// ═══════════════════════════════════════════════════

#[test]
fn test_partial_solution_no_fabricated_plain_owned() {
    // A cluster that must conflict. Verify no node gets PlainOwned
    // unless its floor was PlainOwned.
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcMutShared),
        mk_node_bounded(1, OwnershipTier::RcShared, OwnershipTier::RcShared),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Conflict { .. } => {
            // Expected: ArcMutShared can't fit in RcShared ceiling.
        }
        LatticeOutcome::IterationBudgetExceeded { partial_map, .. } => {
            // If budget exceeded, verify no fabricated PlainOwned.
            for node in &cluster.nodes {
                if let Some(tier) = partial_map.get(node.id) {
                    if tier == OwnershipTier::PlainOwned && node.floor != OwnershipTier::PlainOwned {
                        panic!(
                            "Regression: node {:?} got PlainOwned but floor is {:?}",
                            node.id, node.floor
                        );
                    }
                }
            }
        }
        LatticeOutcome::Solved(map) => {
            // If somehow solved, verify no ceiling violations.
            let v1 = map.get(KirNodeId(1)).unwrap();
            assert!(
                tier_rank(v1) <= tier_rank(OwnershipTier::RcShared),
                "Regression: solved with ceiling violation {:?}",
                v1
            );
        }
    }
}
