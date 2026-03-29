use kobo_ir::SolutionMap;

// --- Types first ---

/// Input to the solver: a graph of ownership constraints extracted from KIR.
/// Populated in v0.8; empty stub for v0.1.
pub struct ConstraintGraph;

/// Time and cluster budget for the solver.
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

/// Result of the solver.
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

// --- Functions ---

/// Solves ownership constraints for the given graph within the given budget.
///
/// Stub at v0.1: no constraints → returns an empty `SolutionMap`.
/// The pipeline calls this unconditionally so the hook exists for v0.8.
pub fn solve(_graph: &ConstraintGraph, _budget: SolverBudget) -> SolveResult {
    SolveResult::Unique(SolutionMap::new())
}
