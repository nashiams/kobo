//! Property-based tests for the Kobo ownership solver.
//!
//! Uses proptest to verify fundamental invariants hold for any random cluster.

use proptest::prelude::*;

use kobo_ir::{KirNodeId, OwnershipTier};
use kobo_migrate::cluster::{Cluster, ClusterId};
use kobo_migrate::constraint_extract::ConstraintNode;
use kobo_migrate::lattice_solve::{lattice_solve, tier_rank, LatticeOutcome};
use kobo_migrate::{ConstraintEdge, ConstraintKind};

// ─────────────────── Arbitrary generators ───────────────────

fn arb_tier() -> impl Strategy<Value = OwnershipTier> {
    prop_oneof![
        Just(OwnershipTier::PlainOwned),
        Just(OwnershipTier::BoxOwned),
        Just(OwnershipTier::RcShared),
        Just(OwnershipTier::ArcShared),
        Just(OwnershipTier::RcMutShared),
        Just(OwnershipTier::ArcMutShared),
        Just(OwnershipTier::Scoped),
    ]
}

fn arb_constraint_kind() -> impl Strategy<Value = ConstraintKind> {
    prop_oneof![
        Just(ConstraintKind::PropagateSharing),
        Just(ConstraintKind::PropagateSend),
        Just(ConstraintKind::MutuallyExclusive),
    ]
}

fn arb_node(id: u32) -> impl Strategy<Value = ConstraintNode> {
    // 70% uncapped, 30% with ceiling >= floor.
    arb_tier().prop_flat_map(move |floor| {
        let floor_rank = tier_rank(floor);
        // Build ceiling options: None or Some(tier) where tier_rank >= floor_rank.
        let ceiling_strat = prop_oneof![
            7 => Just(None),
            3 => arb_tier()
                .prop_filter("ceiling must be >= floor", move |t| tier_rank(*t) >= floor_rank)
                .prop_map(Some),
        ];
        ceiling_strat.prop_map(move |ceiling| ConstraintNode {
            id: KirNodeId(id),
            floor,
            ceiling,
            is_boundary: false,
            binding_name: format!("v{}", id),
        })
    })
}

fn arb_edge(max_nodes: u32) -> impl Strategy<Value = ConstraintEdge> {
    (0..max_nodes, 0..max_nodes, arb_constraint_kind()).prop_map(|(src, tgt, kind)| {
        ConstraintEdge::synthetic(KirNodeId(src), KirNodeId(tgt), kind, "proptest")
    })
}

fn arb_cluster(max_nodes: u32) -> impl Strategy<Value = Cluster> {
    (2..=max_nodes).prop_flat_map(|n| {
        let nodes = (0..n).map(|i| arb_node(i)).collect::<Vec<_>>();
        let max_edges = (n as usize) * 2;
        (nodes, prop::collection::vec(arb_edge(n), 0..=max_edges)).prop_map(|(nodes, edges)| {
            let size = nodes.len();
            Cluster {
                id: ClusterId(0),
                nodes,
                edges: Vec::new(),
                raw_edges: edges,
                size,
            }
        })
    })
}

// ═══════════════════════════════════════════════════
// Property 1: Termination — solver always returns
// ═══════════════════════════════════════════════════

proptest! {
    #[test]
    fn prop_termination(cluster in arb_cluster(50)) {
        // Must return one of the three variants — never hang.
        let _outcome = lattice_solve(&cluster);
    }
}

// ═══════════════════════════════════════════════════
// Property 2: Soundness — solved maps satisfy constraints
// ═══════════════════════════════════════════════════

proptest! {
    #[test]
    fn prop_soundness(cluster in arb_cluster(30)) {
        if let LatticeOutcome::Solved(map) = lattice_solve(&cluster) {
            for edge in &cluster.raw_edges {
                let src = map.get(edge.source);
                let tgt = map.get(edge.target);
                if let (Some(s), Some(t)) = (src, tgt) {
                    match &edge.kind {
                        ConstraintKind::PropagateSharing => {
                            // If source is shared, target must be at least as high.
                            prop_assert!(
                                tier_rank(t) >= tier_rank(s),
                                "PropagateSharing violated: {:?}({:?}) → {:?}({:?})",
                                edge.source, s, edge.target, t
                            );
                        }
                        ConstraintKind::PropagateSend => {
                            // If source needs send, target should be thread-safe.
                            if s.is_thread_safe() {
                                prop_assert!(
                                    t.is_thread_safe(),
                                    "PropagateSend violated: source {:?} is thread-safe but target {:?} is not",
                                    s, t
                                );
                            }
                        }
                        ConstraintKind::MutuallyExclusive => {
                            // Both should not be mutable wrappers.
                            let s_mut = matches!(s, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
                            let t_mut = matches!(t, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
                            prop_assert!(
                                !(s_mut && t_mut),
                                "MutuallyExclusive violated: both {:?} and {:?} are mutable",
                                s, t
                            );
                        }
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════
// Property 3: Floor invariant
// ═══════════════════════════════════════════════════

proptest! {
    #[test]
    fn prop_floor_invariant(cluster in arb_cluster(30)) {
        if let LatticeOutcome::Solved(map) = lattice_solve(&cluster) {
            for node in &cluster.nodes {
                if let Some(tier) = map.get(node.id) {
                    prop_assert!(
                        tier_rank(tier) >= tier_rank(node.floor),
                        "Floor violated: node {:?} solved to {:?} but floor is {:?}",
                        node.id, tier, node.floor
                    );
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════
// Property 4: Ceiling invariant
// ═══════════════════════════════════════════════════

proptest! {
    #[test]
    fn prop_ceiling_invariant(cluster in arb_cluster(30)) {
        if let LatticeOutcome::Solved(map) = lattice_solve(&cluster) {
            for node in &cluster.nodes {
                if let Some(ceiling) = node.ceiling {
                    if let Some(tier) = map.get(node.id) {
                        prop_assert!(
                            tier_rank(tier) <= tier_rank(ceiling),
                            "Ceiling violated: node {:?} solved to {:?} but ceiling is {:?}",
                            node.id, tier, ceiling
                        );
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════
// Property 5: Idempotence — solving twice gives same result
// ═══════════════════════════════════════════════════

proptest! {
    #[test]
    fn prop_idempotence(cluster in arb_cluster(30)) {
        let r1 = lattice_solve(&cluster);
        let r2 = lattice_solve(&cluster);

        match (&r1, &r2) {
            (LatticeOutcome::Solved(m1), LatticeOutcome::Solved(m2)) => {
                for node in &cluster.nodes {
                    prop_assert_eq!(
                        m1.get(node.id),
                        m2.get(node.id),
                        "Idempotence violated for node {:?}",
                        node.id
                    );
                }
            }
            (LatticeOutcome::Conflict { node: n1, .. }, LatticeOutcome::Conflict { node: n2, .. }) => {
                prop_assert_eq!(n1, n2, "Conflict node should be deterministic");
            }
            (LatticeOutcome::IterationBudgetExceeded { .. }, LatticeOutcome::IterationBudgetExceeded { .. }) => {
                // Both exceeded — acceptable.
            }
            _ => {
                prop_assert!(false, "Idempotence violated: different outcome variants");
            }
        }
    }
}
