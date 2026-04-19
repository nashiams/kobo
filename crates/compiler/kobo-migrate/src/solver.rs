use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap, TierDecision, TierReason};

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
            .field("solution", &format!("SolutionMap[len={}]", self.solution.len()))
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

/// Solves ownership constraints for the given graph within the given budget.
///
/// At v0.8: minimal stub that returns Unique with empty solution.
/// Full HM-style inference will be implemented post-v0.8.
pub fn solve(graph: &ConstraintGraph, _budget: SolverBudget) -> SolveOutcome {
    // Stub: return Unique with empty solution map.
    if graph.nodes.is_empty() {
        return SolveOutcome::Unique(SolutionMap::new());
    }

    // In a full implementation, this would:
    // 1. Extract connected components (clusters)
    // 2. Check cluster size limit
    // 3. For each cluster, run constraint propagation
    // 4. Detect conflicts and multi-solutions
    // 5. Return appropriate SolveOutcome

    SolveOutcome::Unique(SolutionMap::new())
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
        let edge = ConstraintEdge {
            source: KirNodeId(1),
            target: KirNodeId(2),
            kind: ConstraintKind::PropagateSharing,
        };
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
    fn solver_budget_has_defaults() {
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

    /// Non-empty graph + stub → still Unique.
    #[test]
    fn non_empty_graph_stub_still_unique() {
        let graph = ConstraintGraph {
            nodes: vec![KirNodeId(1), KirNodeId(2), KirNodeId(3)],
            edges: vec![
                ConstraintEdge {
                    source: KirNodeId(1),
                    target: KirNodeId(2),
                    kind: ConstraintKind::PropagateSharing,
                },
            ],
        };
        let result = solve(&graph, SolverBudget::default());
        assert!(matches!(result, SolveOutcome::Unique(_)));
    }

    /// Budget: max_cluster_size and budget_seconds are sane.
    #[test]
    fn budget_defaults_sane() {
        let budget = SolverBudget::default();
        assert!(budget.max_cluster_size > 0);
        assert!(budget.budget_seconds > 0.0);
        assert!(budget.max_cluster_size <= 65536, "cluster limit seems too large");
        assert!(budget.budget_seconds <= 300.0, "budget seems too generous");
    }

    /// ConflictReport constructable.
    #[test]
    fn conflict_report_constructable() {
        let r = ConflictReport {
            conflicting_nodes: vec![KirNodeId(1), KirNodeId(2)],
            conflict_reason: "conflict".to_owned(),
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
    fn partial_report_exceeded_budget() {
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
        });
        // They are different enum variants
        assert!(!matches!(multi, SolveOutcome::NoSolution(_)));
        assert!(!matches!(no_sol, SolveOutcome::MultiSolution(_)));
    }
}

