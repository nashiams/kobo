mod boundary;
#[cfg(test)]
mod evidence_contract_tests;
mod greedy;
mod solver;

pub use boundary::{detect_crate_boundaries, BoundaryViolation, ExternalCall};
pub use greedy::{greedy_resolve, GreedyConfig, GreedyDiagnostic, GreedyPassResult, GreedyStats};
pub use solver::{
    solve, BoundaryReport, ClusterReport, ConflictReport, ConstraintEdge,
    ConstraintGraph, ConstraintKind, PartialReport, SolutionCandidate, SolveOutcome, SolverBudget,
};

// Legacy exports for backward compatibility
pub use solver::{SolveResult};
