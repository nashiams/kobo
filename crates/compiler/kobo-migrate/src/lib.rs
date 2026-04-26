pub mod backtrack;
mod boundary;
pub mod cache;
pub mod call_graph;
pub mod chooser;
pub mod cluster;
pub mod constraint_extract;
pub mod ctxt;
pub mod decision_class;
pub mod decisions_cache;
pub mod decompose;
#[cfg(test)]
mod evidence_contract_tests;
pub mod explain;
mod greedy;
pub mod lattice_solve;
pub mod parallel;
pub mod profile;
pub mod provenance_comment;
pub mod query;
mod solver;
pub mod summaries;

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
