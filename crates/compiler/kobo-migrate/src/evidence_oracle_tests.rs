use crate::{
    solve, BoundaryReport, ClusterReport, ConflictReport, ConstraintEdge, ConstraintGraph,
    ConstraintKind, PartialReport, SolveOutcome, SolverBudget,
};
use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap};
use std::{collections::BTreeMap, fs, path::Path};

const ORACLE_TIERS: [OwnershipTier; 5] = [
    OwnershipTier::PlainOwned,
    OwnershipTier::RcShared,
    OwnershipTier::ArcShared,
    OwnershipTier::RcMutShared,
    OwnershipTier::ArcMutShared,
];

const SOLVED_LATTICE_TIERS: [OwnershipTier; 7] = [
    OwnershipTier::PlainOwned,
    OwnershipTier::BoxOwned,
    OwnershipTier::RcShared,
    OwnershipTier::ArcShared,
    OwnershipTier::RcMutShared,
    OwnershipTier::ArcMutShared,
    OwnershipTier::Scoped,
];

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum OracleClassification {
    Unique,
    MultiSolution,
    NoSolution,
}

fn residual_graph() -> ConstraintGraph {
    ConstraintGraph {
        nodes: vec![KirNodeId(1), KirNodeId(2)],
        edges: vec![ConstraintEdge::synthetic(
            KirNodeId(1),
            KirNodeId(2),
            ConstraintKind::PropagateSharing,
            "test",
        )],
    }
}

fn outcome_name(outcome: &SolveOutcome) -> &'static str {
    match outcome {
        SolveOutcome::Unique(_) => "Unique",
        SolveOutcome::MultiSolution(_) => "MultiSolution",
        SolveOutcome::NoSolution(_) => "NoSolution",
        SolveOutcome::ClusterTooLarge(_) => "ClusterTooLarge",
        SolveOutcome::BudgetExceeded(_) => "BudgetExceeded",
        SolveOutcome::BoundaryStop(_) => "BoundaryStop",
    }
}

fn join_by_floor_strength(current: OwnershipTier, incoming: OwnershipTier) -> OwnershipTier {
    if incoming.priority() > current.priority() {
        incoming
    } else {
        current
    }
}

fn solved_lattice_rank(tier: OwnershipTier) -> u8 {
    match tier {
        OwnershipTier::PlainOwned => 0,
        OwnershipTier::BoxOwned => 1,
        OwnershipTier::RcShared => 2,
        OwnershipTier::ArcShared => 3,
        OwnershipTier::RcMutShared => 4,
        OwnershipTier::ArcMutShared => 5,
        OwnershipTier::Scoped => 6,
        OwnershipTier::Undecided => panic!("Undecided is not part of the solved ownership lattice"),
    }
}

fn solved_lub(left: OwnershipTier, right: OwnershipTier) -> OwnershipTier {
    if solved_lattice_rank(left) >= solved_lattice_rank(right) {
        left
    } else {
        right
    }
}

fn brute_force_oracle(graph: &ConstraintGraph) -> Vec<BTreeMap<KirNodeId, OwnershipTier>> {
    fn visit(
        graph: &ConstraintGraph,
        index: usize,
        current: &mut BTreeMap<KirNodeId, OwnershipTier>,
        solutions: &mut Vec<BTreeMap<KirNodeId, OwnershipTier>>,
    ) {
        if index == graph.nodes.len() {
            if graph
                .edges
                .iter()
                .all(|edge| oracle_edge_satisfied(edge, current))
            {
                solutions.push(current.clone());
            }
            return;
        }

        let node = graph.nodes[index];
        for tier in ORACLE_TIERS {
            current.insert(node, tier);
            visit(graph, index + 1, current, solutions);
        }
        current.remove(&node);
    }

    let mut solutions = Vec::new();
    visit(graph, 0, &mut BTreeMap::new(), &mut solutions);
    solutions
}

fn oracle_classification(graph: &ConstraintGraph) -> OracleClassification {
    match brute_force_oracle(graph).len() {
        0 => OracleClassification::NoSolution,
        1 => OracleClassification::Unique,
        _ => OracleClassification::MultiSolution,
    }
}

fn oracle_edge_satisfied(
    edge: &ConstraintEdge,
    assignment: &BTreeMap<KirNodeId, OwnershipTier>,
) -> bool {
    let source = assignment
        .get(&edge.source)
        .copied()
        .expect("oracle assignment should contain source node");
    let target = assignment
        .get(&edge.target)
        .copied()
        .expect("oracle assignment should contain target node");

    match edge.kind {
        ConstraintKind::PropagateSharing => !source.is_shared() || target.is_shared(),
        ConstraintKind::PropagateSend => !source.is_thread_safe() || target.is_thread_safe(),
        ConstraintKind::MutuallyExclusive => {
            !(is_mutably_shared(source) && is_mutably_shared(target))
        }
    }
}

fn is_mutably_shared(tier: OwnershipTier) -> bool {
    matches!(
        tier,
        OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared
    )
}

fn assert_solution_covers_graph(graph: &ConstraintGraph, solution: &SolutionMap) {
    assert_eq!(
        solution.len(),
        graph.nodes.len(),
        "unique solver evidence must map exactly every graph node"
    );

    for node in &graph.nodes {
        assert!(
            solution.contains(*node),
            "solver evidence omitted node {:?}",
            node
        );
    }

    for edge in &graph.edges {
        let source = solution
            .get(edge.source)
            .expect("solution should contain edge source");
        let target = solution
            .get(edge.target)
            .expect("solution should contain edge target");
        assert!(
            oracle_edge_satisfied(
                edge,
                &BTreeMap::from([(edge.source, source), (edge.target, target)])
            ),
            "solver solution violates edge {:?}",
            edge
        );
    }
}

fn assert_solver_matches_oracle(graph: ConstraintGraph) {
    let expected = oracle_classification(&graph);
    let outcome = solve(&graph, SolverBudget::default());

    match (expected, outcome) {
        (OracleClassification::Unique, SolveOutcome::Unique(solution)) => {
            assert_solution_covers_graph(&graph, &solution)
        }
        (OracleClassification::MultiSolution, SolveOutcome::MultiSolution(candidates)) => {
            assert!(
                !candidates.is_empty(),
                "multi-solution outcomes must carry executable candidates"
            );
            for candidate in candidates {
                assert_solution_covers_graph(&graph, &candidate.solution);
            }
        }
        (OracleClassification::NoSolution, SolveOutcome::NoSolution(report)) => assert!(
            !report.conflicting_nodes.is_empty() || !report.conflict_reason.is_empty(),
            "no-solution outcomes must carry conflict evidence"
        ),
        (expected, actual) => panic!(
            "solver outcome disagreed with independent oracle: expected {:?}, got {}",
            expected,
            outcome_name(&actual)
        ),
    }
}

fn candidate_solution_signature(solution: &SolutionMap) -> Vec<(KirNodeId, OwnershipTier)> {
    let mut signature: Vec<_> = solution.iter().collect();
    signature.sort_by_key(|(node, tier)| (node.0, *tier));
    signature
}

fn graph(nodes: &[u32], edges: Vec<ConstraintEdge>) -> ConstraintGraph {
    ConstraintGraph {
        nodes: nodes.iter().copied().map(KirNodeId).collect(),
        edges,
    }
}

#[test]
fn oracle_seven_tier_lub_matrix_must_match_solver_lattice() {
    for left in SOLVED_LATTICE_TIERS {
        for right in SOLVED_LATTICE_TIERS {
            let expected = solved_lub(left, right);
            let actual = crate::lattice_solve::lattice_lub(left, right);

            assert_eq!(
                actual, expected,
                "7-tier ownership LUB mismatch for {left:?} and {right:?}"
            );
            assert_eq!(
                actual,
                crate::lattice_solve::lattice_lub(right, left),
                "LUB must be commutative for {left:?} and {right:?}"
            );
            assert!(
                solved_lattice_rank(actual) >= solved_lattice_rank(left)
                    && solved_lattice_rank(actual) >= solved_lattice_rank(right),
                "LUB({left:?}, {right:?}) returned {actual:?}, which is not an upper bound"
            );
        }

        assert_eq!(
            crate::lattice_solve::lattice_lub(left, left),
            left,
            "LUB must be idempotent for {left:?}"
        );
    }
}

#[test]
fn oracle_candidate_tier_ladder_must_cover_solver_lattice() {
    let solver_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/solver.rs");
    let source = fs::read_to_string(&solver_path).expect("solver source should be readable");
    let executable = strip_line_comments(&source);
    let candidate_block = executable
        .split("const CANDIDATE_TIERS")
        .nth(1)
        .and_then(|tail| tail.split_once("];").map(|(block, _)| block))
        .expect("solver must define CANDIDATE_TIERS for oracle checking");

    for tier in SOLVED_LATTICE_TIERS {
        let needle = format!("OwnershipTier::{tier:?}");
        assert!(
            candidate_block.contains(&needle),
            "candidate tier ladder omitted solver tier {tier:?}"
        );
    }
}

#[test]
fn oracle_budget_exceeded_evidence_must_not_collapse_non_empty_graph_to_empty_map() {
    let graph = residual_graph();
    let evidence = crate::solve_with_evidence(
        &graph,
        SolverBudget {
            max_cluster_size: 256,
            budget_seconds: 0.0,
        },
    );

    assert_eq!(evidence.outcome_name, "BudgetExceeded");
    assert_eq!(evidence.node_count, graph.nodes.len());
    assert!(
        !evidence.solution.is_empty(),
        "BudgetExceeded evidence for a non-empty graph must carry partial solver payload, not SolutionMap::new() collapse"
    );
}

#[test]
fn oracle_multisolution_candidates_are_distinct_and_deterministic() {
    let graph = graph(&[1], Vec::new());
    let mut first_signature = None;

    for _ in 0..100 {
        let outcome = solve(&graph, SolverBudget::default());
        let candidates = match outcome {
            SolveOutcome::MultiSolution(candidates) => candidates,
            other => panic!(
                "ambiguous one-node graph must produce MultiSolution, got {}",
                outcome_name(&other)
            ),
        };

        assert!(
            candidates.len() >= 2,
            "MultiSolution must contain real alternatives, not a single picked candidate"
        );

        let signatures: Vec<_> = candidates
            .iter()
            .map(|candidate| candidate_solution_signature(&candidate.solution))
            .collect();
        let unique_signatures: std::collections::BTreeSet<_> = signatures.iter().cloned().collect();
        assert_eq!(
            unique_signatures.len(),
            signatures.len(),
            "MultiSolution candidates must have distinct SolutionMap contents"
        );

        if let Some(expected) = &first_signature {
            assert_eq!(
                &signatures, expected,
                "candidate order must be byte-stable across repeated solves"
            );
        } else {
            first_signature = Some(signatures);
        }
    }
}

#[test]
fn example_non_empty_residual_graph_cannot_earn_empty_unique_credit() {
    let graph = residual_graph();
    let outcome = solve(&graph, SolverBudget::default());

    if let SolveOutcome::Unique(solution) = outcome {
        assert_solution_covers_graph(&graph, &solution);
    }
}

#[test]
fn oracle_generated_small_graphs_must_match_bruteforce_classification() {
    let cases = [
        graph(&[], Vec::new()),
        graph(&[1], Vec::new()),
        graph(&[1, 2], Vec::new()),
        residual_graph(),
        graph(
            &[1, 2, 3],
            vec![
                ConstraintEdge::synthetic(
                    KirNodeId(1),
                    KirNodeId(2),
                    ConstraintKind::PropagateSharing,
                    "test",
                ),
                ConstraintEdge::synthetic(
                    KirNodeId(2),
                    KirNodeId(3),
                    ConstraintKind::PropagateSend,
                    "test",
                ),
            ],
        ),
    ];

    for case in cases {
        assert_solver_matches_oracle(case);
    }
}

#[test]
fn oracle_cluster_limit_must_fire_before_claiming_solution() {
    let graph = graph(&[1, 2, 3], Vec::new());
    let outcome = solve(
        &graph,
        SolverBudget {
            max_cluster_size: 2,
            budget_seconds: 5.0,
        },
    );

    match outcome {
        SolveOutcome::ClusterTooLarge(report) => {
            assert_eq!(report.limit, 2);
            assert!(report.cluster_size >= 3);
            assert_eq!(report.member_nodes.len(), 3);
        }
        other => panic!(
            "oversized cluster must report ClusterTooLarge, got {}",
            outcome_name(&other)
        ),
    }
}

#[test]
fn oracle_zero_budget_must_not_claim_verified_solution() {
    let graph = residual_graph();
    let outcome = solve(
        &graph,
        SolverBudget {
            max_cluster_size: 256,
            budget_seconds: 0.0,
        },
    );

    match outcome {
        SolveOutcome::BudgetExceeded(report) => {
            assert_eq!(report.budget_seconds, 0.0);
            assert!(report.total_count >= graph.nodes.len());
            assert!(report.resolved_count <= report.total_count);
        }
        other => panic!(
            "zero-budget non-empty solve must report BudgetExceeded, got {}",
            outcome_name(&other)
        ),
    }
}

#[test]
fn source_oracle_solver_must_not_be_stub_or_dead_parameter_theater() {
    let solver_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/solver.rs");
    let source = fs::read_to_string(&solver_path).expect("solver source should be readable");
    let executable = strip_line_comments(&source);

    assert!(
        !source.contains("minimal stub")
            && !source.contains("Stub:")
            && !source.contains("Full HM-style inference will be implemented later"),
        "solver source still advertises itself as future/stub work"
    );
    assert!(
        !executable.contains("_budget"),
        "budget must be used as executable evidence, not hidden behind an unused parameter"
    );
    assert!(
        executable.contains("graph.edges") || executable.contains(".edges"),
        "solver must inspect constraint edges, not only node count"
    );
    assert!(
        executable.contains("budget.max_cluster_size"),
        "solver must enforce cluster limits from SolverBudget"
    );
    assert!(
        executable.contains("budget.budget_seconds"),
        "solver must enforce time/search budget from SolverBudget"
    );
}

fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn differential_empty_graph_is_the_only_empty_unique_case() {
    let graph = ConstraintGraph {
        nodes: Vec::new(),
        edges: Vec::new(),
    };

    let outcome = solve(&graph, SolverBudget::default());

    match outcome {
        SolveOutcome::Unique(solution) => assert!(
            solution.is_empty(),
            "empty residual graph should stay empty for greedy-only agreement"
        ),
        other => panic!(
            "empty residual graph should be Unique, got {}",
            outcome_name(&other)
        ),
    }
}

#[test]
fn runtime_outcome_variants_have_distinct_stable_names() {
    let outcomes = [
        SolveOutcome::Unique(SolutionMap::new()),
        SolveOutcome::MultiSolution(Vec::new()),
        SolveOutcome::NoSolution(ConflictReport {
            conflicting_nodes: vec![KirNodeId(1), KirNodeId(2)],
            conflict_reason: "conflicting ownership floors".to_owned(),
            provenance: vec![],
        }),
        SolveOutcome::ClusterTooLarge(ClusterReport {
            cluster_size: 257,
            limit: 256,
            member_nodes: vec![KirNodeId(1)],
        }),
        SolveOutcome::BudgetExceeded(PartialReport {
            resolved_count: 1,
            total_count: 2,
            elapsed_seconds: 6.0,
            budget_seconds: 5.0,
        }),
        SolveOutcome::BoundaryStop(BoundaryReport {
            crossing_node: KirNodeId(1),
            external_crate: "external".to_owned(),
            external_function: "call".to_owned(),
        }),
    ];

    let names: Vec<_> = outcomes.iter().map(outcome_name).collect();
    assert_eq!(
        names,
        vec![
            "Unique",
            "MultiSolution",
            "NoSolution",
            "ClusterTooLarge",
            "BudgetExceeded",
            "BoundaryStop",
        ],
        "SolveOutcome variants must stay semantically distinct until projection"
    );
}

#[test]
fn property_floor_strength_chain_never_decreases() {
    let floors = [
        OwnershipTier::PlainOwned,
        OwnershipTier::RcShared,
        OwnershipTier::ArcShared,
        OwnershipTier::RcMutShared,
        OwnershipTier::ArcMutShared,
    ];

    for pair in floors.windows(2) {
        assert!(
            pair[1].priority() >= pair[0].priority(),
            "stronger ownership floor {:?} must not rank below {:?}",
            pair[1],
            pair[0]
        );
    }
}

#[test]
fn property_stronger_constraints_never_lower_a_tier() {
    let floors = [
        OwnershipTier::PlainOwned,
        OwnershipTier::RcShared,
        OwnershipTier::ArcShared,
        OwnershipTier::RcMutShared,
        OwnershipTier::ArcMutShared,
    ];

    for current in floors {
        for incoming in floors {
            let joined = join_by_floor_strength(current, incoming);
            assert!(
                joined.priority() >= current.priority(),
                "joining {:?} with {:?} lowered to {:?}",
                current,
                incoming,
                joined
            );
        }
    }
}

#[test]
fn runtime_partial_report_keeps_budget_exhaustion_separate_from_conflict() {
    let report = PartialReport {
        resolved_count: 1,
        total_count: 3,
        elapsed_seconds: 5.5,
        budget_seconds: 5.0,
    };

    debug_assert!(report.resolved_count <= report.total_count);
    debug_assert!(report.elapsed_seconds >= report.budget_seconds);

    let outcome = SolveOutcome::BudgetExceeded(report);
    assert_eq!(outcome_name(&outcome), "BudgetExceeded");
}
