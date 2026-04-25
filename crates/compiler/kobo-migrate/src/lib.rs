mod boundary;
#[cfg(test)]
mod evidence_contract_tests;
mod greedy;
mod solver;

pub use boundary::{detect_crate_boundaries, BoundaryViolation, ExternalCall};
pub use greedy::{greedy_resolve, GreedyConfig, GreedyDiagnostic, GreedyPassResult, GreedyStats};
pub use solver::{
    build_kir_constraint_graph, graph_fingerprint, solve, solve_with_evidence,
    BoundaryReport, ClusterReport, ConflictReport, ConstraintEdge,
    ConstraintGraph, ConstraintKind, PartialReport, SolutionCandidate, SolveOutcome,
    SolverBudget, SolverEvidence,
};

// Legacy exports for backward compatibility
pub use solver::SolveResult;
