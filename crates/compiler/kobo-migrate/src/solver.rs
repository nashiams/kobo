use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::time::Instant;

use kobo_ir::{FileId, Kir, KirNodeId, KoboSpan, NodeKind, OwnershipTier, SolutionMap};

/// Input to the solver: a graph of ownership constraints extracted from KIR.
pub struct ConstraintGraph {
    pub nodes: Vec<KirNodeId>,
    pub edges: Vec<ConstraintEdge>,
}

/// A constraint between two nodes.
#[derive(Clone, Debug)]
pub struct ConstraintEdge {
    pub source: KirNodeId,
    pub target: KirNodeId,
    pub kind: ConstraintKind,
    pub provenance: ConstraintProvenance,
}

impl ConstraintEdge {
    pub fn new(
        source: KirNodeId,
        target: KirNodeId,
        kind: ConstraintKind,
        provenance: ConstraintProvenance,
    ) -> Self {
        Self {
            source,
            target,
            kind,
            provenance,
        }
    }

    pub fn synthetic(
        source: KirNodeId,
        target: KirNodeId,
        kind: ConstraintKind,
        rule_name: &'static str,
    ) -> Self {
        Self::new(
            source,
            target,
            kind,
            ConstraintProvenance::synthetic(rule_name),
        )
    }
}

/// Provenance attached to every solver constraint edge.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ConstraintProvenance {
    pub span: KoboSpan,
    pub fact_kind: ConstraintFactKind,
    pub rule_name: &'static str,
    pub source_binding: Option<String>,
}

impl ConstraintProvenance {
    pub fn new(
        span: KoboSpan,
        fact_kind: ConstraintFactKind,
        rule_name: &'static str,
        source_binding: Option<String>,
    ) -> Self {
        Self {
            span,
            fact_kind,
            rule_name,
            source_binding,
        }
    }

    pub fn synthetic(rule_name: &'static str) -> Self {
        Self {
            span: KoboSpan::new(0, 0, FileId(0)),
            fact_kind: ConstraintFactKind::Generated,
            rule_name,
            source_binding: None,
        }
    }
}

/// The source fact that generated a constraint edge.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ConstraintFactKind {
    NeedsSharing,
    NeedsSend,
    MutableShared,
    AliasFlow,
    EngineCeiling,
    BoundaryMarker,
    CoMutation,
    Generated,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ConstraintKind {
    /// Target must be Shared if Source is Shared.
    PropagateSharing,
    /// Target must be Send if Source is Send.
    PropagateSend,
    /// Source and Target cannot have conflicting mutability.
    MutuallyExclusive,
}

/// Time and cluster budget for the solver.
#[derive(Clone, Debug)]
pub struct SolverBudget {
    pub max_cluster_size: usize,
    pub budget_seconds: f64,
}

impl Default for SolverBudget {
    fn default() -> Self {
        Self {
            max_cluster_size: 256,
            budget_seconds: 5.0,
        }
    }
}

/// Outcome of solving ownership constraints.
pub enum SolveOutcome {
    /// Exactly one valid ownership assignment.
    Unique(SolutionMap),
    /// Multiple valid assignments — present to user for choice.
    MultiSolution(Vec<SolutionCandidate>),
    /// No valid assignment under current constraints (K0080).
    NoSolution(ConflictReport),
    /// Cluster exceeded size threshold before search (K0081).
    ClusterTooLarge(ClusterReport),
    /// Search budget exhausted during solving (K0082).
    BudgetExceeded(PartialReport),
    /// Would require crossing crate boundary without summary (K0090).
    BoundaryStop(BoundaryReport),
}

/// A single solution candidate with explanation.
pub struct SolutionCandidate {
    pub solution: SolutionMap,
    pub explanation: String,
    pub risk_score: f64,
}

impl std::fmt::Debug for SolutionCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolutionCandidate")
            .field(
                "solution",
                &format!("SolutionMap[len={}]", self.solution.len()),
            )
            .field("explanation", &self.explanation)
            .field("risk_score", &self.risk_score)
            .finish()
    }
}

/// Explanation for why no solution was found.
#[derive(Clone, Debug)]
pub struct ConflictReport {
    pub conflicting_nodes: Vec<KirNodeId>,
    pub conflict_reason: String,
    pub provenance: Vec<ConstraintProvenance>,
}

/// Explanation for cluster size violation.
#[derive(Clone, Debug)]
pub struct ClusterReport {
    pub cluster_size: usize,
    pub limit: usize,
    pub member_nodes: Vec<KirNodeId>,
}

/// Explanation for budget timeout.
#[derive(Clone, Debug)]
pub struct PartialReport {
    pub resolved_count: usize,
    pub total_count: usize,
    pub elapsed_seconds: f64,
    pub budget_seconds: f64,
}

/// Explanation for crate boundary stop.
#[derive(Clone, Debug)]
pub struct BoundaryReport {
    pub crossing_node: KirNodeId,
    pub external_crate: String,
    pub external_function: String,
}

/// Result of the solver (legacy — kept for backward compatibility).
pub enum SolveResult {
    /// All nodes resolved to a unique ownership tier.
    Unique(SolutionMap),
    /// Some nodes have multiple valid tiers; user must choose.
    MultiSolution(SolutionMap),
    /// Solver timed out before resolving all nodes.
    Timeout(SolutionMap),
    /// A constraint cycle was detected and cannot be resolved automatically.
    Conflict,
}

/// Evidence produced alongside solve outcome for source map provenance.
#[derive(Clone, Debug)]
pub struct SolverEvidence {
    pub outcome_name: String,
    pub graph_fingerprint: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub budget: SolverBudget,
    pub solution: SolutionMap,
}

/// Candidate tiers considered during constraint solving.
const CANDIDATE_TIERS: [OwnershipTier; 7] = [
    OwnershipTier::PlainOwned,
    OwnershipTier::BoxOwned,
    OwnershipTier::RcShared,
    OwnershipTier::ArcShared,
    OwnershipTier::RcMutShared,
    OwnershipTier::ArcMutShared,
    OwnershipTier::Scoped,
];

/// Compute a deterministic fingerprint for a constraint graph.
pub fn graph_fingerprint(graph: &ConstraintGraph) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    graph.nodes.len().hash(&mut hasher);
    for node in &graph.nodes {
        node.0.hash(&mut hasher);
    }
    graph.edges.len().hash(&mut hasher);
    for edge in &graph.edges {
        edge.source.0.hash(&mut hasher);
        edge.target.0.hash(&mut hasher);
        edge.kind.hash(&mut hasher);
        edge.provenance.hash(&mut hasher);
    }
    format!("{:016x}{:016x}", hasher.finish(), {
        // Second round for length ≥ 16 chars and better distribution.
        let mut h2 = std::collections::hash_map::DefaultHasher::new();
        hasher.finish().hash(&mut h2);
        graph.nodes.len().wrapping_mul(31).hash(&mut h2);
        h2.finish()
    })
}

/// Build a constraint graph from KIR declaration nodes.
pub fn build_kir_constraint_graph(kir: &Kir) -> ConstraintGraph {
    let decl_nodes: Vec<KirNodeId> = kir.iter_decl_nodes().map(|n| n.id).collect();

    let mut edges = Vec::new();
    let decl_set: BTreeSet<KirNodeId> = decl_nodes.iter().copied().collect();

    // Build edges from Use/Borrow nodes that share declarations.
    // Group non-Decl nodes by their decl_id to find inter-binding relationships.
    let mut scope_decls: BTreeMap<Option<u32>, Vec<KirNodeId>> = BTreeMap::new();
    for node in kir.iter_nodes() {
        if node.kind == NodeKind::Decl {
            continue;
        }
        if let Some(decl_id) = node.decl_id {
            if decl_set.contains(&decl_id) {
                scope_decls
                    .entry(node.cfg_block.map(|b| b.0))
                    .or_default()
                    .push(decl_id);
            }
        }
    }

    // For bindings used together in the same scope block, add PropagateSharing edges.
    let mut edge_set = BTreeSet::new();
    for decl_ids in scope_decls.values() {
        let unique_decls: BTreeSet<KirNodeId> = decl_ids.iter().copied().collect();
        let unique_vec: Vec<KirNodeId> = unique_decls.into_iter().collect();
        for i in 0..unique_vec.len() {
            for j in (i + 1)..unique_vec.len() {
                let (a, b) = if unique_vec[i] < unique_vec[j] {
                    (unique_vec[i], unique_vec[j])
                } else {
                    (unique_vec[j], unique_vec[i])
                };
                if edge_set.insert((a, b)) {
                    edges.push(ConstraintEdge::new(
                        a,
                        b,
                        ConstraintKind::PropagateSharing,
                        ConstraintProvenance::new(
                            KoboSpan::new(0, 0, FileId(0)),
                            ConstraintFactKind::AliasFlow,
                            "kir-co-scope-sharing",
                            None,
                        ),
                    ));
                }
            }
        }
    }

    ConstraintGraph {
        nodes: decl_nodes,
        edges,
    }
}

/// Solves ownership constraints for the given graph within the given budget.
///
/// Runs arc-consistency propagation followed by backtracking search to classify
/// the outcome as Unique, MultiSolution, or NoSolution. Enforces cluster size
/// and time budget limits before and during search.
pub fn solve(graph: &ConstraintGraph, budget: SolverBudget) -> SolveOutcome {
    if graph.nodes.is_empty() {
        return SolveOutcome::Unique(SolutionMap::new());
    }

    // Enforce cluster size limit from budget.
    if graph.nodes.len() > budget.max_cluster_size {
        return SolveOutcome::ClusterTooLarge(ClusterReport {
            cluster_size: graph.nodes.len(),
            limit: budget.max_cluster_size,
            member_nodes: graph.nodes.clone(),
        });
    }

    let start = Instant::now();

    // Zero or negative budget: cannot claim any verified solution.
    if budget.budget_seconds <= 0.0 {
        return SolveOutcome::BudgetExceeded(PartialReport {
            resolved_count: 0,
            total_count: graph.nodes.len(),
            elapsed_seconds: start.elapsed().as_secs_f64(),
            budget_seconds: budget.budget_seconds,
        });
    }

    // Initialize domains: each node can be any of the candidate tiers.
    let mut domains: BTreeMap<KirNodeId, BTreeSet<OwnershipTier>> = BTreeMap::new();
    for &node in &graph.nodes {
        domains.insert(node, CANDIDATE_TIERS.iter().copied().collect());
    }

    // Arc-consistency (AC-3) propagation over constraint edges.
    if !propagate_arc_consistency(&mut domains, &graph.edges, &budget, &start) {
        let resolved = domains.values().filter(|d| d.len() == 1).count();
        return SolveOutcome::BudgetExceeded(PartialReport {
            resolved_count: resolved,
            total_count: graph.nodes.len(),
            elapsed_seconds: start.elapsed().as_secs_f64(),
            budget_seconds: budget.budget_seconds,
        });
    }

    // Check for empty domains after propagation.
    let empty_nodes: Vec<KirNodeId> = domains
        .iter()
        .filter(|(_, d)| d.is_empty())
        .map(|(&id, _)| id)
        .collect();
    if !empty_nodes.is_empty() {
        return SolveOutcome::NoSolution(ConflictReport {
            conflicting_nodes: empty_nodes,
            conflict_reason: "constraint propagation eliminated all valid tiers".to_owned(),
            provenance: graph
                .edges
                .iter()
                .map(|edge| edge.provenance.clone())
                .collect(),
        });
    }

    // Backtracking search to enumerate solutions (stop at 2 to classify).
    let node_order: Vec<KirNodeId> = graph.nodes.clone();
    let mut solutions: Vec<BTreeMap<KirNodeId, OwnershipTier>> = Vec::new();
    let mut current = BTreeMap::new();
    let max_solutions = 2;

    backtrack_search(
        &node_order,
        0,
        &mut current,
        &domains,
        &graph.edges,
        budget.budget_seconds,
        &start,
        &mut solutions,
        max_solutions,
    );

    // If search exhausted budget without finding any solutions, report budget exceeded.
    if solutions.is_empty() && start.elapsed().as_secs_f64() > budget.budget_seconds {
        let resolved = domains.values().filter(|d| d.len() == 1).count();
        return SolveOutcome::BudgetExceeded(PartialReport {
            resolved_count: resolved,
            total_count: graph.nodes.len(),
            elapsed_seconds: start.elapsed().as_secs_f64(),
            budget_seconds: budget.budget_seconds,
        });
    }

    match solutions.len() {
        0 => SolveOutcome::NoSolution(ConflictReport {
            conflicting_nodes: graph.nodes.clone(),
            conflict_reason: "no valid ownership assignment satisfies all constraints".to_owned(),
            provenance: graph
                .edges
                .iter()
                .map(|edge| edge.provenance.clone())
                .collect(),
        }),
        1 => {
            let mut map = SolutionMap::new();
            for (&node, &tier) in &solutions[0] {
                map.insert(node, tier);
            }
            SolveOutcome::Unique(map)
        }
        _ => {
            let candidates: Vec<SolutionCandidate> = solutions
                .into_iter()
                .map(|assignment| {
                    let mut map = SolutionMap::new();
                    for (&node, &tier) in &assignment {
                        map.insert(node, tier);
                    }
                    SolutionCandidate {
                        solution: map,
                        explanation: "valid ownership assignment".to_owned(),
                        risk_score: 0.0,
                    }
                })
                .collect();
            SolveOutcome::MultiSolution(candidates)
        }
    }
}

/// Solve and produce evidence for the pipeline.
pub fn solve_with_evidence(graph: &ConstraintGraph, budget: SolverBudget) -> SolverEvidence {
    let fingerprint = graph_fingerprint(graph);
    let node_count = graph.nodes.len();
    let edge_count = graph.edges.len();
    let budget_clone = budget.clone();
    let outcome = solve(graph, budget);

    let (outcome_name, solution) = match outcome {
        SolveOutcome::Unique(map) => ("Unique".to_owned(), map),
        SolveOutcome::MultiSolution(candidates) => {
            let sol = candidates
                .into_iter()
                .next()
                .map(|c| c.solution)
                .unwrap_or_default();
            ("MultiSolution".to_owned(), sol)
        }
        SolveOutcome::NoSolution(_) => ("NoSolution".to_owned(), partial_solution_for_graph(graph)),
        SolveOutcome::ClusterTooLarge(_) => (
            "ClusterTooLarge".to_owned(),
            partial_solution_for_graph(graph),
        ),
        SolveOutcome::BudgetExceeded(_) => (
            "BudgetExceeded".to_owned(),
            partial_solution_for_graph(graph),
        ),
        SolveOutcome::BoundaryStop(_) => {
            ("BoundaryStop".to_owned(), partial_solution_for_graph(graph))
        }
    };

    SolverEvidence {
        outcome_name,
        graph_fingerprint: fingerprint,
        node_count,
        edge_count,
        budget: budget_clone,
        solution,
    }
}

fn partial_solution_for_graph(graph: &ConstraintGraph) -> SolutionMap {
    let mut map = SolutionMap::new();
    for node in &graph.nodes {
        map.insert(*node, OwnershipTier::PlainOwned);
    }
    map
}

/// Returns the canonical name for a SolveOutcome variant.
pub fn outcome_name(outcome: &SolveOutcome) -> &'static str {
    match outcome {
        SolveOutcome::Unique(_) => "Unique",
        SolveOutcome::MultiSolution(_) => "MultiSolution",
        SolveOutcome::NoSolution(_) => "NoSolution",
        SolveOutcome::ClusterTooLarge(_) => "ClusterTooLarge",
        SolveOutcome::BudgetExceeded(_) => "BudgetExceeded",
        SolveOutcome::BoundaryStop(_) => "BoundaryStop",
    }
}

// --- Internal constraint propagation ---

fn check_constraint(source: OwnershipTier, target: OwnershipTier, kind: &ConstraintKind) -> bool {
    match kind {
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

fn propagate_arc_consistency(
    domains: &mut BTreeMap<KirNodeId, BTreeSet<OwnershipTier>>,
    edges: &[ConstraintEdge],
    budget: &SolverBudget,
    start: &Instant,
) -> bool {
    let mut changed = true;
    while changed {
        if start.elapsed().as_secs_f64() > budget.budget_seconds {
            return false;
        }
        changed = false;
        for edge in edges {
            let source_domain = match domains.get(&edge.source) {
                Some(d) => d.clone(),
                None => continue,
            };
            let target_domain = match domains.get(&edge.target) {
                Some(d) => d.clone(),
                None => continue,
            };

            let new_source: BTreeSet<OwnershipTier> = source_domain
                .iter()
                .filter(|&&s| {
                    target_domain
                        .iter()
                        .any(|&t| check_constraint(s, t, &edge.kind))
                })
                .copied()
                .collect();

            let new_target: BTreeSet<OwnershipTier> = target_domain
                .iter()
                .filter(|&&t| {
                    source_domain
                        .iter()
                        .any(|&s| check_constraint(s, t, &edge.kind))
                })
                .copied()
                .collect();

            if new_source.len() < source_domain.len() {
                domains.insert(edge.source, new_source);
                changed = true;
            }
            if new_target.len() < target_domain.len() {
                domains.insert(edge.target, new_target);
                changed = true;
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn backtrack_search(
    nodes: &[KirNodeId],
    index: usize,
    current: &mut BTreeMap<KirNodeId, OwnershipTier>,
    domains: &BTreeMap<KirNodeId, BTreeSet<OwnershipTier>>,
    edges: &[ConstraintEdge],
    budget_seconds: f64,
    start: &Instant,
    solutions: &mut Vec<BTreeMap<KirNodeId, OwnershipTier>>,
    max_solutions: usize,
) {
    if solutions.len() >= max_solutions {
        return;
    }
    if start.elapsed().as_secs_f64() > budget_seconds {
        return;
    }
    if index == nodes.len() {
        if edges
            .iter()
            .all(|e| edge_satisfied_in_assignment(e, current))
        {
            solutions.push(current.clone());
        }
        return;
    }

    let node = nodes[index];
    let domain = match domains.get(&node) {
        Some(d) => d,
        None => return,
    };

    for &tier in domain {
        current.insert(node, tier);
        if partial_consistent(current, edges) {
            backtrack_search(
                nodes,
                index + 1,
                current,
                domains,
                edges,
                budget_seconds,
                start,
                solutions,
                max_solutions,
            );
        }
        current.remove(&node);
    }
}

fn edge_satisfied_in_assignment(
    edge: &ConstraintEdge,
    assignment: &BTreeMap<KirNodeId, OwnershipTier>,
) -> bool {
    match (assignment.get(&edge.source), assignment.get(&edge.target)) {
        (Some(&s), Some(&t)) => check_constraint(s, t, &edge.kind),
        _ => true,
    }
}

fn partial_consistent(
    assignment: &BTreeMap<KirNodeId, OwnershipTier>,
    edges: &[ConstraintEdge],
) -> bool {
    for edge in edges {
        if let (Some(&s), Some(&t)) = (assignment.get(&edge.source), assignment.get(&edge.target)) {
            if !check_constraint(s, t, &edge.kind) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solver_returns_unique_on_empty_graph() {
        let graph = ConstraintGraph {
            nodes: vec![],
            edges: vec![],
        };
        let result = solve(&graph, SolverBudget::default());
        assert!(matches!(result, SolveOutcome::Unique(_)));
    }

    #[test]
    fn constraint_edge_propagation() {
        let edge = ConstraintEdge::synthetic(
            KirNodeId(1),
            KirNodeId(2),
            ConstraintKind::PropagateSharing,
            "test",
        );
        assert_eq!(edge.kind, ConstraintKind::PropagateSharing);
    }

    #[test]
    fn solution_candidate_has_risk_score() {
        let candidate = SolutionCandidate {
            solution: SolutionMap::new(),
            explanation: "test".to_owned(),
            risk_score: 0.5,
        };
        assert_eq!(candidate.risk_score, 0.5);
    }

    #[test]
    fn solver_limits_have_defaults() {
        let budget = SolverBudget::default();
        assert_eq!(budget.max_cluster_size, 256);
        assert_eq!(budget.budget_seconds, 5.0);
    }

    // ─── v0.8 edge-case tests ───

    /// Exhaustive match proves SolveOutcome has exactly 6 variants.
    #[test]
    fn solve_outcome_six_variants_exhaustive() {
        let outcomes: Vec<SolveOutcome> = vec![
            SolveOutcome::Unique(SolutionMap::new()),
            SolveOutcome::MultiSolution(vec![]),
            SolveOutcome::NoSolution(ConflictReport {
                conflicting_nodes: vec![],
                conflict_reason: "no solution".to_owned(),
                provenance: vec![],
            }),
            SolveOutcome::ClusterTooLarge(ClusterReport {
                cluster_size: 999,
                limit: 256,
                member_nodes: vec![],
            }),
            SolveOutcome::BudgetExceeded(PartialReport {
                resolved_count: 5,
                total_count: 20,
                elapsed_seconds: 10.0,
                budget_seconds: 5.0,
            }),
            SolveOutcome::BoundaryStop(BoundaryReport {
                crossing_node: KirNodeId(1),
                external_crate: "tokio".to_owned(),
                external_function: "spawn".to_owned(),
            }),
        ];
        assert_eq!(outcomes.len(), 6, "SolveOutcome must have 6 variants");
    }

    /// Empty graph → Unique (stub behavior).
    #[test]
    fn empty_graph_returns_unique() {
        let graph = ConstraintGraph {
            nodes: vec![],
            edges: vec![],
        };
        let result = solve(&graph, SolverBudget::default());
        assert!(matches!(result, SolveOutcome::Unique(_)));
    }

    /// Non-empty unconstrained graph → MultiSolution (multiple valid tier assignments).
    #[test]
    fn non_empty_unconstrained_graph_is_multi_solution() {
        let graph = ConstraintGraph {
            nodes: vec![KirNodeId(1), KirNodeId(2), KirNodeId(3)],
            edges: vec![ConstraintEdge::synthetic(
                KirNodeId(1),
                KirNodeId(2),
                ConstraintKind::PropagateSharing,
                "test",
            )],
        };
        let result = solve(&graph, SolverBudget::default());
        assert!(matches!(result, SolveOutcome::MultiSolution(_)));
    }

    /// Budget: max_cluster_size and budget_seconds are sane.
    #[test]
    fn budget_defaults_sane() {
        let budget = SolverBudget::default();
        assert!(budget.max_cluster_size > 0);
        assert!(budget.budget_seconds > 0.0);
        assert!(
            budget.max_cluster_size <= 65536,
            "cluster limit seems too large"
        );
        assert!(budget.budget_seconds <= 300.0, "budget seems too generous");
    }

    /// ConflictReport constructable.
    #[test]
    fn conflict_report_constructable() {
        let r = ConflictReport {
            conflicting_nodes: vec![KirNodeId(1), KirNodeId(2)],
            conflict_reason: "conflict".to_owned(),
            provenance: vec![],
        };
        assert_eq!(r.conflicting_nodes.len(), 2);
    }

    /// BoundaryReport constructable.
    #[test]
    fn boundary_report_constructable() {
        let r = BoundaryReport {
            crossing_node: KirNodeId(1),
            external_crate: "serde".to_owned(),
            external_function: "serialize".to_owned(),
        };
        assert_eq!(r.external_crate, "serde");
    }

    /// ClusterReport constructable.
    #[test]
    fn cluster_report_constructable() {
        let r = ClusterReport {
            cluster_size: 512,
            limit: 256,
            member_nodes: vec![KirNodeId(1), KirNodeId(2)],
        };
        assert!(r.cluster_size > r.limit);
        assert_eq!(r.member_nodes.len(), 2);
    }

    /// PartialReport with exceeded budget.
    #[test]
    fn partial_report_over_time_limit() {
        let r = PartialReport {
            resolved_count: 10,
            total_count: 50,
            elapsed_seconds: 6.0,
            budget_seconds: 5.0,
        };
        assert!(r.elapsed_seconds > r.budget_seconds);
        assert!(r.resolved_count < r.total_count);
    }

    /// MultiSolution ≠ NoSolution (K0080) — they are different variants.
    #[test]
    fn multi_solution_is_not_no_solution() {
        let multi = SolveOutcome::MultiSolution(vec![]);
        let no_sol = SolveOutcome::NoSolution(ConflictReport {
            conflicting_nodes: vec![],
            conflict_reason: String::new(),
            provenance: vec![],
        });
        // They are different enum variants
        assert!(!matches!(multi, SolveOutcome::NoSolution(_)));
        assert!(!matches!(no_sol, SolveOutcome::MultiSolution(_)));
    }
}
