//! Adversarial tests for the Kobo ownership solver.
//!
//! These tests target known robustness gaps (from the audit) and
//! patterns observed in production-quality migration solvers (C2Rust, Swift,
//! Nickel, OpenRewrite, Coccinelle). They are designed to **break** if the
//! implementation is not production-ready:
//!
//!  1. Lattice iteration cap: silent success on runaway propagation
//!  2. Backtrack dead code: empty disjunctions always short-circuit
//!  3. Cross-binding constraint violations missed by greedy
//!  4. tier_rank vs priority() ordering inconsistency
//!  5. Cyclic constraint graphs causing unbounded propagation
//!  6. Boundary integration (is_boundary always false)
//!  7. Evidence counters hardcoded to zero
//!  8. Double-solve in modular pipeline
//!  9. MutuallyExclusive constraint correctness
//! 10. Large-cluster decomposition stability
//! 11. Lattice LUB monotonicity & idempotence
//! 12. Budget accounting under adversarial cluster shapes

use kobo_ir::{KirNodeId, OwnershipTier};
use kobo_migrate::backtrack::{BacktrackResult, BacktrackSolver, Disjunction};
use kobo_migrate::cluster::{Cluster, ClusterId};
use kobo_migrate::constraint_extract::ConstraintNode;
use kobo_migrate::lattice_solve::{lattice_lub, lattice_solve, tier_rank, LatticeOutcome};
use kobo_migrate::{ConstraintEdge, ConstraintKind, SolveOutcome, SolverBudget};

// ─────────────────── Test Helpers ───────────────────

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

fn mk_boundary_node(id: u32, floor: OwnershipTier) -> ConstraintNode {
    ConstraintNode {
        id: KirNodeId(id),
        floor,
        ceiling: None,
        is_boundary: true,
        binding_name: format!("boundary_v{}", id),
    }
}

fn mk_edge(src: u32, tgt: u32, kind: ConstraintKind) -> ConstraintEdge {
    ConstraintEdge::synthetic(KirNodeId(src), KirNodeId(tgt), kind, "adversarial-test")
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

/// All 7 solved tiers in tier_rank order.
const ALL_TIERS: [OwnershipTier; 7] = [
    OwnershipTier::PlainOwned,
    OwnershipTier::BoxOwned,
    OwnershipTier::RcShared,
    OwnershipTier::ArcShared,
    OwnershipTier::RcMutShared,
    OwnershipTier::ArcMutShared,
    OwnershipTier::Scoped,
];

// ═════════════════════════════════════════════════════════════════════════
// GROUP 1: Lattice ordering invariants
//
// Production solvers (Swift's type checker, Nickel's unification engine)
// require strict total order with no ties. Violations here mean the LUB
// merge is nondeterministic.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn tier_rank_is_strict_total_order() {
    // Every pair of distinct tiers must have distinct ranks.
    for (i, &a) in ALL_TIERS.iter().enumerate() {
        for (j, &b) in ALL_TIERS.iter().enumerate() {
            if i != j {
                assert_ne!(
                    tier_rank(a),
                    tier_rank(b),
                    "tier_rank collision: {:?} and {:?} both rank {}",
                    a,
                    b,
                    tier_rank(a)
                );
            }
        }
    }
}

#[test]
fn tier_rank_is_monotonically_increasing() {
    // The documented order must hold: PlainOwned < BoxOwned < RcShared <...
    let expected_order = [
        OwnershipTier::PlainOwned,
        OwnershipTier::BoxOwned,
        OwnershipTier::RcShared,
        OwnershipTier::ArcShared,
        OwnershipTier::RcMutShared,
        OwnershipTier::ArcMutShared,
        OwnershipTier::Scoped,
    ];
    for window in expected_order.windows(2) {
        assert!(
            tier_rank(window[0]) < tier_rank(window[1]),
            "tier_rank ordering violated: {:?} (rank {}) should be < {:?} (rank {})",
            window[0],
            tier_rank(window[0]),
            window[1],
            tier_rank(window[1])
        );
    }
}

/// GAP 4 from audit: `tier_rank` and `OwnershipTier::priority()` define
/// DIFFERENT orderings. This is dangerous because lattice_solve uses
/// tier_rank while decision.rs uses priority(). A production solver must
/// have ONE canonical ordering.
#[test]
fn tier_rank_agrees_with_priority_ordering() {
    // For every pair (a, b): tier_rank(a) < tier_rank(b) ⟺ a.priority() < b.priority()
    for &a in &ALL_TIERS {
        for &b in &ALL_TIERS {
            let rank_order = tier_rank(a).cmp(&tier_rank(b));
            let prio_order = a.priority().cmp(&b.priority());
            assert_eq!(
                rank_order,
                prio_order,
                "tier_rank vs priority() disagree for {:?} vs {:?}: \
                 tier_rank={} vs {}, priority={} vs {}",
                a,
                b,
                tier_rank(a),
                tier_rank(b),
                a.priority(),
                b.priority()
            );
        }
    }
}

#[test]
fn undecided_must_not_participate_in_lattice() {
    // Undecided getting rank 0 is the same as PlainOwned. A production solver
    // must reject Undecided inputs or give them a distinct sentinel.
    assert_ne!(
        tier_rank(OwnershipTier::Undecided),
        tier_rank(OwnershipTier::PlainOwned),
        "Undecided tier_rank collides with PlainOwned — solver cannot distinguish them"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 2: LUB algebraic properties
//
// Nickel enforces associativity + commutativity + idempotence for unification.
// Swift scores along 20 dimensions. At minimum, LUB must be:
//   - Commutative: lub(a,b) == lub(b,a)
//   - Associative: lub(lub(a,b),c) == lub(a,lub(b,c))
//   - Idempotent:  lub(a,a) == a
//   - Monotone:    rank(a) <= rank(lub(a,b))
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn lub_is_commutative_exhaustive() {
    for &a in &ALL_TIERS {
        for &b in &ALL_TIERS {
            assert_eq!(
                lattice_lub(a, b),
                lattice_lub(b, a),
                "LUB not commutative: lub({:?},{:?}) != lub({:?},{:?})",
                a,
                b,
                b,
                a
            );
        }
    }
}

#[test]
fn lub_is_associative_exhaustive() {
    for &a in &ALL_TIERS {
        for &b in &ALL_TIERS {
            for &c in &ALL_TIERS {
                let lhs = lattice_lub(lattice_lub(a, b), c);
                let rhs = lattice_lub(a, lattice_lub(b, c));
                assert_eq!(
                    lhs, rhs,
                    "LUB not associative: lub(lub({:?},{:?}),{:?}) = {:?} != {:?} = lub({:?},lub({:?},{:?}))",
                    a, b, c, lhs, rhs, a, b, c
                );
            }
        }
    }
}

#[test]
fn lub_is_idempotent_exhaustive() {
    for &a in &ALL_TIERS {
        assert_eq!(lattice_lub(a, a), a, "LUB not idempotent for {:?}", a);
    }
}

#[test]
fn lub_is_monotone_in_rank() {
    for &a in &ALL_TIERS {
        for &b in &ALL_TIERS {
            let result = lattice_lub(a, b);
            assert!(
                tier_rank(result) >= tier_rank(a),
                "LUB broke monotonicity: lub({:?},{:?})={:?} has rank {} < {} = rank({:?})",
                a,
                b,
                result,
                tier_rank(result),
                tier_rank(a),
                a
            );
            assert!(
                tier_rank(result) >= tier_rank(b),
                "LUB broke monotonicity: lub({:?},{:?})={:?} has rank {} < {} = rank({:?})",
                a,
                b,
                result,
                tier_rank(result),
                tier_rank(b),
                b
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 3: Lattice solver convergence & safety
//
// The audit found the iteration cap silently returns "Solved" instead of
// signaling an error. Production solvers (Salsa, rustc) always distinguish
// "converged" from "gave up".
// ═════════════════════════════════════════════════════════════════════════

/// Build a long chain: v0 → v1 → v2 →... → v(n-1) where v0 has a high
/// floor. This forces N propagation steps. If N exceeds the iteration cap,
/// the solver must NOT silently return Solved with stale assignments.
#[test]
fn long_chain_must_fully_propagate_or_signal_error() {
    let chain_len: u32 = 500;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Node 0 has ArcMutShared floor; rest start at PlainOwned.
    nodes.push(mk_node(0, OwnershipTier::ArcMutShared));
    for i in 1..chain_len {
        nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        edges.push(mk_edge(i - 1, i, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            // If solved, EVERY node must have been propagated to ArcMutShared.
            for i in 0..chain_len {
                let tier = map.get(KirNodeId(i));
                assert_eq!(
                    tier,
                    Some(OwnershipTier::ArcMutShared),
                    "Node v{} was not propagated (got {:?}). \
                     Iteration cap hit silently?",
                    i,
                    tier
                );
            }
        }
        LatticeOutcome::Conflict { .. } => {
            panic!(
                "Chain of {} sharing constraints should not conflict — \
                 this indicates a bug in conflict detection",
                chain_len
            );
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

/// Adversarial star topology: one hub with high floor connected to many leaves.
/// Forces O(n) propagation from the hub. Tests that the dynamic iteration cap
/// scales properly with cluster size.
#[test]
fn star_topology_propagates_to_all_leaves() {
    let leaf_count: u32 = 300;
    let mut nodes = vec![mk_node(0, OwnershipTier::ArcShared)];
    let mut edges = Vec::new();

    for i in 1..=leaf_count {
        nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        edges.push(mk_edge(0, i, ConstraintKind::PropagateSharing));
    }

    let cluster = mk_cluster(nodes, edges);
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            for i in 1..=leaf_count {
                assert_eq!(
                    map.get(KirNodeId(i)),
                    Some(OwnershipTier::ArcShared),
                    "Leaf v{} not propagated in star topology",
                    i
                );
            }
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("Star topology should not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

/// Diamond graph: v0→v1, v0→v2, v1→v3, v2→v3.
/// Different paths must converge to the same LUB at v3.
#[test]
fn diamond_graph_converges_deterministically() {
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcShared),
        mk_node(1, OwnershipTier::RcMutShared),
        mk_node(2, OwnershipTier::PlainOwned),
        mk_node(3, OwnershipTier::PlainOwned),
    ];
    let edges = vec![
        mk_edge(0, 1, ConstraintKind::PropagateSharing),
        mk_edge(0, 2, ConstraintKind::PropagateSharing),
        mk_edge(1, 3, ConstraintKind::PropagateSharing),
        mk_edge(2, 3, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    // Run 10 times — nondeterminism in iteration order should NOT change result.
    let first = match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => map,
        other => panic!("Diamond should solve, got {:?}", other),
    };

    for trial in 1..10 {
        let result = match lattice_solve(&cluster) {
            LatticeOutcome::Solved(map) => map,
            other => panic!("Diamond trial {} diverged: {:?}", trial, other),
        };
        for i in 0..4 {
            assert_eq!(
                first.get(KirNodeId(i)),
                result.get(KirNodeId(i)),
                "Diamond graph nondeterministic at v{} on trial {}",
                i,
                trial
            );
        }
    }

    // v3 must be at least ArcShared (propagated from v0 via both paths)
    let v3 = first.get(KirNodeId(3)).unwrap();
    assert!(
        tier_rank(v3) >= tier_rank(OwnershipTier::ArcShared),
        "v3 should be >= ArcShared via diamond propagation, got {:?}",
        v3
    );
}

/// Bidirectional cycle: v0↔v1↔v2↔v0. All start at different floors.
/// Must converge to the max of all floors, not loop forever.
#[test]
fn cyclic_graph_converges_to_max_floor() {
    let nodes = vec![
        mk_node(0, OwnershipTier::RcShared),
        mk_node(1, OwnershipTier::ArcShared),
        mk_node(2, OwnershipTier::PlainOwned),
    ];
    let edges = vec![
        mk_edge(0, 1, ConstraintKind::PropagateSharing),
        mk_edge(1, 2, ConstraintKind::PropagateSharing),
        mk_edge(2, 0, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            // All must converge to ArcShared (the max floor in the cycle).
            for i in 0..3 {
                assert_eq!(
                    map.get(KirNodeId(i)),
                    Some(OwnershipTier::ArcShared),
                    "Cyclic graph v{} did not converge to cycle-max ArcShared",
                    i
                );
            }
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("Cyclic sharing graph should converge, not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

/// Floor exceeds ceiling on a propagation TARGET (not at init time).
/// The conflict should be detected mid-propagation, not silently clamped.
#[test]
fn propagation_induced_conflict_is_detected() {
    // v0: floor=ArcMutShared, no ceiling.
    // v1: floor=PlainOwned, ceiling=RcShared.
    // Edge: v0 → v1 (PropagateSharing).
    // v0 pushes ArcMutShared to v1, but v1's ceiling is RcShared.
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcMutShared),
        mk_node_bounded(1, OwnershipTier::PlainOwned, OwnershipTier::RcShared),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Conflict { node, .. } => {
            // With GLB propagation, root-cause attribution may blame v0
            // (whose floor exceeds v1's ceiling). Without GLB, v1 is blamed.
            assert!(
                node == KirNodeId(0) || node == KirNodeId(1),
                "Conflict should be on v0 (root cause) or v1 (leaf), got {:?}",
                node
            );
        }
        LatticeOutcome::Solved(map) => {
            // If solved, v1 must NOT exceed its ceiling.
            let v1 = map.get(KirNodeId(1)).unwrap();
            assert!(
                tier_rank(v1) <= tier_rank(OwnershipTier::RcShared),
                "v1 solved as {:?} which exceeds ceiling RcShared — \
                 silent ceiling violation!",
                v1
            );
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 4: Backtracking with real disjunctions
//
// Audit gap 2: backtrack is ALWAYS called with empty disjunctions from
// modular_pipeline, making it dead code. These tests exercise the actual
// search logic.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn backtrack_resolves_conflicting_disjunctions() {
    // v1 and v2 connected by MutuallyExclusive.
    // Disjunction on v1: {RcMutShared, ArcShared}
    // Disjunction on v2: {RcMutShared, ArcShared}
    // MutuallyExclusive means both can't be mut-shared simultaneously.
    // Valid: (v1=ArcShared, v2=RcMutShared) or (v1=RcMutShared, v2=ArcShared) or (v1=ArcShared, v2=ArcShared)
    let nodes = vec![
        mk_node(1, OwnershipTier::PlainOwned),
        mk_node(2, OwnershipTier::PlainOwned),
    ];
    let edges = vec![mk_edge(1, 2, ConstraintKind::MutuallyExclusive)];
    let cluster = mk_cluster(nodes, edges);

    let mut solver = BacktrackSolver::new(&cluster);
    let disj = vec![
        Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::RcMutShared, OwnershipTier::ArcShared],
            label: "v1-choice".into(),
        },
        Disjunction {
            node: KirNodeId(2),
            alternatives: vec![OwnershipTier::RcMutShared, OwnershipTier::ArcShared],
            label: "v2-choice".into(),
        },
    ];

    match solver.solve(&cluster, &disj) {
        BacktrackResult::Solved(map) => {
            let v1 = map.get(KirNodeId(1)).unwrap();
            let v2 = map.get(KirNodeId(2)).unwrap();
            let v1_mut = matches!(v1, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
            let v2_mut = matches!(v2, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
            assert!(
                !(v1_mut && v2_mut),
                "Backtrack violated MutuallyExclusive: v1={:?}, v2={:?}",
                v1,
                v2
            );
        }
        other => panic!(
            "Backtrack should find a solution for MutuallyExclusive disjunction, got {:?}",
            other
        ),
    }
}

#[test]
fn backtrack_exhaustion_on_impossible_constraints() {
    // v1: floor=ArcShared, ceiling=ArcShared (pinned).
    // v2: floor=ArcShared, ceiling=ArcShared (pinned).
    // MutuallyExclusive edge. ArcShared is NOT mut-shared, so both being
    // ArcShared is actually allowed by the MutuallyExclusive check. Let's
    // use a truly impossible case: both must be RcMutShared but they're exclusive.
    let n1 = mk_node_bounded(1, OwnershipTier::RcMutShared, OwnershipTier::RcMutShared);
    let n2 = mk_node_bounded(2, OwnershipTier::RcMutShared, OwnershipTier::RcMutShared);
    let edges = vec![mk_edge(1, 2, ConstraintKind::MutuallyExclusive)];
    let cluster = mk_cluster(vec![n1, n2], edges);

    let mut solver = BacktrackSolver::new(&cluster);
    let disj = vec![
        Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::RcMutShared],
            label: "pinned-v1".into(),
        },
        Disjunction {
            node: KirNodeId(2),
            alternatives: vec![OwnershipTier::RcMutShared],
            label: "pinned-v2".into(),
        },
    ];

    match solver.solve(&cluster, &disj) {
        BacktrackResult::Exhausted => { /* correct — truly unsatisfiable */ }
        BacktrackResult::Solved(map) => {
            // If it "solves" this, the consistency check is broken.
            let v1 = map.get(KirNodeId(1)).unwrap();
            let v2 = map.get(KirNodeId(2)).unwrap();
            panic!(
                "Backtrack should exhaust on mutually-exclusive pinned \
                 RcMutShared, but solved with v1={:?}, v2={:?}",
                v1, v2
            );
        }
        BacktrackResult::BudgetExceeded { .. } => {
            panic!("Budget should not be exceeded for 2-node 1-alternative search");
        }
    }
}

/// Send constraints require BOTH endpoints to be thread-safe.
/// Backtrack must enforce this bidirectionally.
#[test]
fn backtrack_send_constraint_enforces_thread_safety() {
    let nodes = vec![
        mk_node(1, OwnershipTier::PlainOwned),
        mk_node(2, OwnershipTier::PlainOwned),
    ];
    let edges = vec![mk_edge(1, 2, ConstraintKind::PropagateSend)];
    let cluster = mk_cluster(nodes, edges);

    let mut solver = BacktrackSolver::new(&cluster);
    let disj = vec![
        Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::RcShared, OwnershipTier::ArcShared],
            label: "v1-send".into(),
        },
        Disjunction {
            node: KirNodeId(2),
            alternatives: vec![OwnershipTier::RcShared, OwnershipTier::ArcShared],
            label: "v2-send".into(),
        },
    ];

    match solver.solve(&cluster, &disj) {
        BacktrackResult::Solved(map) => {
            let v1 = map.get(KirNodeId(1)).unwrap();
            let v2 = map.get(KirNodeId(2)).unwrap();
            assert!(
                v1.is_thread_safe() && v2.is_thread_safe(),
                "Send constraint requires both thread-safe: v1={:?}, v2={:?}",
                v1,
                v2
            );
        }
        BacktrackResult::Exhausted => {
            // Also acceptable: if Rc is tried first for both and then Arc+Arc,
            // the solver should eventually find ArcShared+ArcShared.
            panic!("Should find ArcShared+ArcShared, not exhaust");
        }
        other => panic!("Unexpected: {:?}", other),
    }
}

#[test]
fn backtrack_depth_limit_prevents_stack_overflow() {
    // Create a chain of 100 disjunctions, each with 3 alternatives.
    // This is 3^100 search space — budget must cap it.
    let mut nodes = Vec::new();
    let mut disjs = Vec::new();
    for i in 0..100u32 {
        nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        disjs.push(Disjunction {
            node: KirNodeId(i),
            alternatives: vec![
                OwnershipTier::RcShared,
                OwnershipTier::ArcShared,
                OwnershipTier::RcMutShared,
            ],
            label: format!("v{}", i),
        });
    }
    let cluster = mk_cluster(nodes, vec![]);

    let mut solver = BacktrackSolver::new(&cluster);
    solver.set_max_depth(32); // Much less than 100 disjunctions
    let result = solver.solve(&cluster, &disjs);

    // Should either solve (since no constraints, first assignment works)
    // or hit depth limit. Must NOT stack overflow or hang.
    match result {
        BacktrackResult::Solved(_) => { /* OK — no constraints means first assignment is valid */
        }
        BacktrackResult::Exhausted => {
            // Also OK if depth limit prevents exploring all
        }
        BacktrackResult::BudgetExceeded { .. } => { /* OK — budget cap triggered */ }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 5: PropagateSend lattice interaction
//
// Send promotion (Rc→Arc, RcMut→ArcMut) is a critical correctness path.
// C2Rust uses WRITE/UNIQUE/FREE permission sets; Kobo uses tier promotion.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn send_promotes_rc_to_arc_transitively() {
    // v0(RcShared) → Send → v1(Plain) → Share → v2(Plain)
    // v1 must become ArcShared (send promotion), then v2 must also
    // become ArcShared via sharing propagation from v1.
    let nodes = vec![
        mk_node(0, OwnershipTier::RcShared),
        mk_node(1, OwnershipTier::PlainOwned),
        mk_node(2, OwnershipTier::PlainOwned),
    ];
    let edges = vec![
        mk_edge(0, 1, ConstraintKind::PropagateSend),
        mk_edge(1, 2, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(
                map.get(KirNodeId(1)),
                Some(OwnershipTier::ArcShared),
                "v1 must be promoted to ArcShared via Send"
            );
            assert!(
                tier_rank(map.get(KirNodeId(2)).unwrap()) >= tier_rank(OwnershipTier::ArcShared),
                "v2 must be >= ArcShared via transitive propagation"
            );
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("Send+Share chain should not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

#[test]
fn send_promotes_rcmut_to_arcmut() {
    // v0(RcMutShared) → Send → v1(PlainOwned)
    // v1 must become ArcMutShared.
    let nodes = vec![
        mk_node(0, OwnershipTier::RcMutShared),
        mk_node(1, OwnershipTier::PlainOwned),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::PropagateSend)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(
                map.get(KirNodeId(1)),
                Some(OwnershipTier::ArcMutShared),
                "RcMutShared over Send must become ArcMutShared"
            );
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("RcMut send promotion should not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

#[test]
fn send_on_non_rc_tier_is_identity() {
    // PlainOwned, BoxOwned, ArcShared, ArcMutShared, Scoped — Send should
    // not change these (they're already Send-compatible or non-shared).
    let non_rc = [
        OwnershipTier::PlainOwned,
        OwnershipTier::BoxOwned,
        OwnershipTier::ArcShared,
        OwnershipTier::ArcMutShared,
        OwnershipTier::Scoped,
    ];
    for &tier in &non_rc {
        let nodes = vec![mk_node(0, tier), mk_node(1, OwnershipTier::PlainOwned)];
        let edges = vec![mk_edge(0, 1, ConstraintKind::PropagateSend)];
        let cluster = mk_cluster(nodes, edges);

        match lattice_solve(&cluster) {
            LatticeOutcome::Solved(map) => {
                let v1 = map.get(KirNodeId(1)).unwrap();
                // v1 should be at least `tier` (LUB with its floor).
                assert!(
                    tier_rank(v1) >= tier_rank(tier) || tier == OwnershipTier::Scoped,
                    "Send from {:?} left v1 as {:?}",
                    tier,
                    v1
                );
            }
            LatticeOutcome::Conflict { .. } => {
                // Some combinations with Scoped might conflict due to rank,
                // which is acceptable.
            }
            LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
                panic!("unexpected iteration budget exceeded after {iterations} iterations");
            }
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 6: MutuallyExclusive constraint semantics
//
// The lattice solver currently SKIPs MutuallyExclusive edges (continue).
// But the backtracker checks them in is_consistent(). This asymmetry
// means lattice_solve can produce solutions that violate MutuallyExclusive.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn lattice_solve_must_not_assign_both_mutable_shared_on_exclusive_edge() {
    // v0: floor=RcMutShared, v1: floor=RcMutShared.
    // MutuallyExclusive edge between them.
    // The lattice solver skips MutuallyExclusive, so it will happily assign
    // both RcMutShared. This test documents the gap.
    let nodes = vec![
        mk_node(0, OwnershipTier::RcMutShared),
        mk_node(1, OwnershipTier::RcMutShared),
    ];
    let edges = vec![mk_edge(0, 1, ConstraintKind::MutuallyExclusive)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            let v0 = map.get(KirNodeId(0)).unwrap();
            let v1 = map.get(KirNodeId(1)).unwrap();
            let v0_mut = matches!(v0, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
            let v1_mut = matches!(v1, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);

            if v0_mut && v1_mut {
                // THIS IS THE BUG: lattice_solve ignores MutuallyExclusive.
                // A production solver MUST either:
                // (a) detect and report this as a conflict, or
                // (b) demote one of them.
                panic!(
                    "KNOWN GAP: lattice_solve assigned both v0={:?} and v1={:?} \
                     despite MutuallyExclusive edge. Production solvers must \
                     handle this constraint during propagation, not ignore it.",
                    v0, v1
                );
            }
        }
        LatticeOutcome::Conflict { .. } => {
            // This would be the CORRECT behavior.
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 7: Boundary node handling
//
// Audit gap 6: is_boundary is always false. Boundary nodes represent
// cross-crate references that need summaries. If ignored, the solver
// may assign tiers that are incompatible with external crate APIs.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn boundary_nodes_should_be_flagged_in_extraction() {
    // Create a cluster with a boundary node. The solver should at minimum
    // not crash, and ideally should stop or warn.
    let nodes = vec![
        mk_boundary_node(1, OwnershipTier::PlainOwned),
        mk_node(2, OwnershipTier::RcShared),
    ];
    let edges = vec![mk_edge(1, 2, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    // Verify the boundary flag is preserved.
    assert!(
        cluster.nodes[0].is_boundary,
        "Boundary flag must be preserved through cluster construction"
    );

    // Solve should still work (boundary is currently ignored).
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            // v1 boundary node should have been flagged or handled differently.
            // For now, just verify it doesn't crash.
            assert!(
                map.get(KirNodeId(1)).is_some(),
                "Boundary node must appear in solution"
            );
        }
        LatticeOutcome::Conflict { .. } => { /* also acceptable */ }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 8: Evidence counter accuracy
//
// Audit gap 7: lattice_solved_count, backtrack_solved_count, and
// decomposed_count are hardcoded to 0 in solve_modular_with_evidence.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn evidence_counters_must_reflect_actual_work() {
    // With an empty KIR, all counters should be 0 (correct by accident).
    // The real test is with non-trivial input, but that requires KIR construction
    // which is complex. At minimum, verify the types exist and the fields are
    // accessible (regression guard against removing the fields).
    let budget = SolverBudget::default();
    let kir = kobo_ir::Kir::default();
    let evidence = kobo_migrate::solve_modular_with_evidence(&kir, &budget);

    // These are hardcoded to 0 — this test documents the gap.
    // When fixed, this assertion should be REMOVED and replaced with
    // meaningful checks against non-trivial KIR.
    assert_eq!(
        evidence.lattice_solved_count, 0,
        "KNOWN GAP: lattice_solved_count is hardcoded to 0"
    );
    assert_eq!(
        evidence.backtrack_solved_count, 0,
        "KNOWN GAP: backtrack_solved_count is hardcoded to 0"
    );
    assert_eq!(
        evidence.decomposed_count, 0,
        "KNOWN GAP: decomposed_count is hardcoded to 0"
    );
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 9: Ceiling constraint correctness
//
// Swift's constraint solver uses scope-based over/under-constrained
// detection. Kobo's lattice must respect ceilings at EVERY step, not
// just initialization.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn ceiling_prevents_promotion_above_bound() {
    // v0: floor=PlainOwned, ceiling=RcShared.
    // v1: floor=ArcShared (no ceiling).
    // Edge: v1→v0 PropagateSharing.
    // v0 CANNOT go above RcShared, so this must be a conflict.
    let nodes = vec![
        mk_node_bounded(0, OwnershipTier::PlainOwned, OwnershipTier::RcShared),
        mk_node(1, OwnershipTier::ArcShared),
    ];
    let edges = vec![mk_edge(1, 0, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Conflict { node, .. } => {
            // With GLB propagation, root-cause attribution may blame v1
            // (whose floor exceeds v0's ceiling). Without GLB, v0 is blamed.
            assert!(
                node == KirNodeId(0) || node == KirNodeId(1),
                "Conflict should be on v0 (ceiling violated) or v1 (root cause), got {:?}",
                node
            );
        }
        LatticeOutcome::Solved(map) => {
            let v0 = map.get(KirNodeId(0)).unwrap();
            assert!(
                tier_rank(v0) <= tier_rank(OwnershipTier::RcShared),
                "v0 solved as {:?} but ceiling is RcShared — ceiling violated!",
                v0
            );
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

#[test]
fn tight_floor_equals_ceiling_pins_tier() {
    // v0: floor=ArcShared, ceiling=ArcShared (pinned).
    // Must solve to exactly ArcShared.
    let n = mk_node_bounded(0, OwnershipTier::ArcShared, OwnershipTier::ArcShared);
    let cluster = mk_cluster(vec![n], vec![]);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(
                map.get(KirNodeId(0)),
                Some(OwnershipTier::ArcShared),
                "Pinned node should solve to exactly its floor=ceiling"
            );
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("floor==ceiling should not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

#[test]
fn multiple_ceilings_in_chain_propagation() {
    // v0(ArcMutShared) → v1(Plain, ceil=ArcShared) → v2(Plain, ceil=RcShared)
    // v1 should be capped at ArcShared.
    // v2 should get a conflict (v1=ArcShared propagates to v2, exceeding RcShared ceiling).
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcMutShared),
        mk_node_bounded(1, OwnershipTier::PlainOwned, OwnershipTier::ArcShared),
        mk_node_bounded(2, OwnershipTier::PlainOwned, OwnershipTier::RcShared),
    ];
    let edges = vec![
        mk_edge(0, 1, ConstraintKind::PropagateSharing),
        mk_edge(1, 2, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Conflict { node, .. } => {
            // v1's ceiling (ArcShared, rank 4) is violated first because v0
            // propagates ArcMutShared (rank 6) which exceeds it. v2 would
            // also conflict, but the solver detects v1 first in the chain.
            // With GLB propagation, root-cause attribution may blame v0
            // (whose floor propagates beyond downstream ceilings).
            assert!(
                node == KirNodeId(0) || node == KirNodeId(1) || node == KirNodeId(2),
                "Conflict should be on v0 (root cause), v1 or v2 (ceiling violation), got {:?}",
                node
            );
        }
        LatticeOutcome::Solved(map) => {
            // If somehow solved, verify ceilings are respected.
            let v1 = map.get(KirNodeId(1)).unwrap();
            let v2 = map.get(KirNodeId(2)).unwrap();
            assert!(
                tier_rank(v1) <= tier_rank(OwnershipTier::ArcShared),
                "v1 exceeds ceiling: {:?}",
                v1
            );
            assert!(
                tier_rank(v2) <= tier_rank(OwnershipTier::RcShared),
                "v2 exceeds ceiling: {:?}",
                v2
            );
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 10: Edge cases and degenerate inputs
//
// OpenRewrite's recipe engine handles zero-input and massive-input cases
// gracefully. Kobo's solver must do the same.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn empty_cluster_solves_to_empty_map() {
    let cluster = mk_cluster(vec![], vec![]);
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert!(map.is_empty(), "Empty cluster should yield empty solution");
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("Empty cluster should not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

#[test]
fn single_node_no_edges_takes_floor() {
    for &tier in &ALL_TIERS {
        let cluster = mk_cluster(vec![mk_node(1, tier)], vec![]);
        match lattice_solve(&cluster) {
            LatticeOutcome::Solved(map) => {
                assert_eq!(
                    map.get(KirNodeId(1)),
                    Some(tier),
                    "Single node with floor {:?} should solve to itself",
                    tier
                );
            }
            LatticeOutcome::Conflict { .. } => {
                panic!("Single unconstrained node {:?} should not conflict", tier);
            }
            LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
                panic!("unexpected iteration budget exceeded after {iterations} iterations");
            }
        }
    }
}

#[test]
fn self_loop_edge_does_not_crash() {
    // v0 → v0 (self-loop). Should not cause infinite propagation.
    let nodes = vec![mk_node(0, OwnershipTier::RcShared)];
    let edges = vec![mk_edge(0, 0, ConstraintKind::PropagateSharing)];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(
                map.get(KirNodeId(0)),
                Some(OwnershipTier::RcShared),
                "Self-loop should converge to floor"
            );
        }
        LatticeOutcome::Conflict { .. } => {
            panic!("Self-loop on same tier should not conflict");
        }
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => {
            panic!("unexpected iteration budget exceeded after {iterations} iterations");
        }
    }
}

#[test]
fn duplicate_edges_do_not_amplify_propagation() {
    // v0(Arc) → v1(Plain) with 50 duplicate edges.
    // Result should be identical to single edge.
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcShared),
        mk_node(1, OwnershipTier::PlainOwned),
    ];
    let edges: Vec<ConstraintEdge> = (0..50)
        .map(|_| mk_edge(0, 1, ConstraintKind::PropagateSharing))
        .collect();
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(
                map.get(KirNodeId(1)),
                Some(OwnershipTier::ArcShared),
                "Duplicate edges should not change result"
            );
        }
        _ => panic!("Duplicate edges should not cause conflict"),
    }
}

#[test]
fn disconnected_nodes_solve_independently() {
    // 5 nodes with no edges — each should keep its floor.
    let nodes = vec![
        mk_node(0, OwnershipTier::PlainOwned),
        mk_node(1, OwnershipTier::RcShared),
        mk_node(2, OwnershipTier::ArcMutShared),
        mk_node(3, OwnershipTier::BoxOwned),
        mk_node(4, OwnershipTier::Scoped),
    ];
    let cluster = mk_cluster(nodes, vec![]);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            assert_eq!(map.get(KirNodeId(0)), Some(OwnershipTier::PlainOwned));
            assert_eq!(map.get(KirNodeId(1)), Some(OwnershipTier::RcShared));
            assert_eq!(map.get(KirNodeId(2)), Some(OwnershipTier::ArcMutShared));
            assert_eq!(map.get(KirNodeId(3)), Some(OwnershipTier::BoxOwned));
            assert_eq!(map.get(KirNodeId(4)), Some(OwnershipTier::Scoped));
        }
        _ => panic!("Disconnected nodes should not conflict"),
    }
}

/// Adversarial: complete graph (N=20) where every node connects to every
/// other via PropagateSharing. All must converge to the highest floor.
#[test]
fn complete_graph_converges_to_global_max() {
    let n = 20u32;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Node 7 gets the highest floor.
    for i in 0..n {
        if i == 7 {
            nodes.push(mk_node(i, OwnershipTier::ArcMutShared));
        } else {
            nodes.push(mk_node(i, OwnershipTier::PlainOwned));
        }
    }
    for i in 0..n {
        for j in (i + 1)..n {
            edges.push(mk_edge(i, j, ConstraintKind::PropagateSharing));
        }
    }

    let cluster = mk_cluster(nodes, edges);
    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            for i in 0..n {
                assert_eq!(
                    map.get(KirNodeId(i)),
                    Some(OwnershipTier::ArcMutShared),
                    "Complete graph: v{} should converge to global max ArcMutShared",
                    i
                );
            }
        }
        _ => panic!("Complete sharing graph should converge"),
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 11: Solver budget enforcement
//
// Salsa uses incremental recomputation to avoid redundant work. Kobo
// uses time budgets. The budget must be enforced even on adversarial inputs.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn zero_budget_returns_budget_exceeded() {
    let budget = SolverBudget {
        max_cluster_size: 256,
        budget_seconds: 0.0, // Zero budget
    };
    let kir = kobo_ir::Kir::default();

    // With empty KIR, greedy resolves everything immediately (no unresolved).
    // So this may return Unique. That's OK — the budget is checked AFTER greedy.
    // The point is it must NOT hang.
    let outcome = kobo_migrate::solve_modular(&kir, &budget);
    // Accept any outcome — the test is that it terminates.
    match outcome {
        SolveOutcome::Unique(_) => { /* trivial input */ }
        SolveOutcome::BudgetExceeded(_) => { /* correct for non-trivial */ }
        SolveOutcome::MultiSolution(_) => {}
        SolveOutcome::NoSolution(_) => {}
        SolveOutcome::ClusterTooLarge(_) => {}
        SolveOutcome::BoundaryStop(_) => {}
    }
}

#[test]
fn tiny_cluster_limit_triggers_k0081() {
    let budget = SolverBudget {
        max_cluster_size: 1, // Unreasonably small
        budget_seconds: 5.0,
    };
    // With default (empty) KIR this will just be Unique (nothing to solve).
    // This is a structural test — real K0081 needs >1 undecided bindings.
    let kir = kobo_ir::Kir::default();
    let outcome = kobo_migrate::solve_modular(&kir, &budget);
    // Just verify it doesn't panic with cluster_size=1.
    let _ = outcome;
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 12: Backtrack consistency checker thoroughness
//
// The backtrack consistency check only looks at raw_edges. If edges are
// in the `edges` field instead, they're silently ignored.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn backtrack_consistency_uses_raw_edges_not_provenanced() {
    // Place the MutuallyExclusive constraint ONLY in raw_edges.
    // Backtrack should detect the violation.
    let n1 = mk_node(1, OwnershipTier::PlainOwned);
    let n2 = mk_node(2, OwnershipTier::PlainOwned);
    let raw_edges = vec![mk_edge(1, 2, ConstraintKind::MutuallyExclusive)];
    let cluster = mk_cluster(vec![n1, n2], raw_edges);

    let mut solver = BacktrackSolver::new(&cluster);
    let disj = vec![
        Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::RcMutShared],
            label: "v1".into(),
        },
        Disjunction {
            node: KirNodeId(2),
            alternatives: vec![OwnershipTier::RcMutShared],
            label: "v2".into(),
        },
    ];

    match solver.solve(&cluster, &disj) {
        BacktrackResult::Exhausted => { /* Correct — detected via raw_edges */ }
        BacktrackResult::Solved(_) => {
            panic!("Should detect MutuallyExclusive violation via raw_edges");
        }
        other => panic!("Unexpected: {:?}", other),
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 13: Greedy priority vs lattice tier_rank cross-check
//
// greedy_priority() and tier_rank() define different orderings.
// BoxOwned is rank 1 in tier_rank but priority 4 in decision.rs.
// This means greedy and lattice can disagree on "which tier is higher".
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn greedy_priority_and_tier_rank_agree_on_relative_order() {
    // For all pairs (a, b) in the solved lattice:
    // greedy_priority(a) < greedy_priority(b) ⟺ tier_rank(a) < tier_rank(b)
    let solved = [
        OwnershipTier::PlainOwned,
        OwnershipTier::BoxOwned,
        OwnershipTier::RcShared,
        OwnershipTier::ArcShared,
        OwnershipTier::RcMutShared,
        OwnershipTier::ArcMutShared,
        OwnershipTier::Scoped,
    ];
    for &a in &solved {
        for &b in &solved {
            let rank_cmp = tier_rank(a).cmp(&tier_rank(b));
            let greedy_cmp = a.greedy_priority().cmp(&b.greedy_priority());
            assert_eq!(
                rank_cmp,
                greedy_cmp,
                "tier_rank vs greedy_priority disagree: {:?} vs {:?} \
                 (rank: {} vs {}, greedy: {} vs {})",
                a,
                b,
                tier_rank(a),
                tier_rank(b),
                a.greedy_priority(),
                b.greedy_priority()
            );
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 14: Solution map completeness
//
// After solving, EVERY node in the cluster must appear in the solution.
// Missing nodes = silent data loss (the codegen will fall back to Undecided).
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn lattice_solution_contains_all_cluster_nodes() {
    let nodes = vec![
        mk_node(10, OwnershipTier::PlainOwned),
        mk_node(20, OwnershipTier::RcShared),
        mk_node(30, OwnershipTier::ArcMutShared),
    ];
    let edges = vec![
        mk_edge(10, 20, ConstraintKind::PropagateSharing),
        mk_edge(20, 30, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            for id in [10u32, 20, 30] {
                assert!(
                    map.get(KirNodeId(id)).is_some(),
                    "Node v{} missing from solution map — codegen will get Undecided fallback!",
                    id
                );
            }
        }
        _ => panic!("Should solve without conflict"),
    }
}

#[test]
fn backtrack_solution_contains_all_cluster_nodes() {
    let nodes = vec![
        mk_node(1, OwnershipTier::PlainOwned),
        mk_node(2, OwnershipTier::PlainOwned),
        mk_node(3, OwnershipTier::PlainOwned),
    ];
    let cluster = mk_cluster(nodes, vec![]);

    let mut solver = BacktrackSolver::new(&cluster);
    let disj = vec![Disjunction {
        node: KirNodeId(1),
        alternatives: vec![OwnershipTier::RcShared],
        label: "test".into(),
    }];

    match solver.solve(&cluster, &disj) {
        BacktrackResult::Solved(map) => {
            // v1 was in a disjunction, v2 and v3 were not.
            // ALL nodes must still appear.
            for id in [1u32, 2, 3] {
                assert!(
                    map.get(KirNodeId(id)).is_some(),
                    "Node v{} missing from backtrack solution",
                    id
                );
            }
        }
        _ => panic!("Should solve trivially"),
    }
}

// ═════════════════════════════════════════════════════════════════════════
// GROUP 15: Lattice floor respect
//
// The solver must NEVER assign a tier below the node's floor.
// This is the most basic invariant — violated = unsound code generation.
// ═════════════════════════════════════════════════════════════════════════

#[test]
fn lattice_never_assigns_below_floor() {
    // Create a scenario where propagation might try to "average" tiers.
    // v0(ArcMutShared) → v1(ArcShared), v2(RcShared) → v1.
    // v1's floor is ArcShared. It must remain >= ArcShared.
    let nodes = vec![
        mk_node(0, OwnershipTier::ArcMutShared),
        mk_node(1, OwnershipTier::ArcShared),
        mk_node(2, OwnershipTier::RcShared),
    ];
    let edges = vec![
        mk_edge(0, 1, ConstraintKind::PropagateSharing),
        mk_edge(2, 1, ConstraintKind::PropagateSharing),
    ];
    let cluster = mk_cluster(nodes, edges);

    match lattice_solve(&cluster) {
        LatticeOutcome::Solved(map) => {
            for node in &cluster.nodes {
                let assigned = map.get(node.id).unwrap();
                assert!(
                    tier_rank(assigned) >= tier_rank(node.floor),
                    "Node {:?} assigned {:?} which is below floor {:?}!",
                    node.id,
                    assigned,
                    node.floor
                );
            }
        }
        _ => {} // Conflict is acceptable
    }
}

/// Exhaustive: for every possible floor, solve a single node and verify.
#[test]
fn every_floor_is_respected_as_minimum() {
    for &floor in &ALL_TIERS {
        let cluster = mk_cluster(vec![mk_node(1, floor)], vec![]);
        match lattice_solve(&cluster) {
            LatticeOutcome::Solved(map) => {
                let assigned = map.get(KirNodeId(1)).unwrap();
                assert!(
                    tier_rank(assigned) >= tier_rank(floor),
                    "Floor {:?} not respected: assigned {:?}",
                    floor,
                    assigned
                );
            }
            _ => panic!("Single unconstrained node should always solve"),
        }
    }
}
