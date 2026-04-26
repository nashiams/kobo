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
mod modular_pipeline;
pub mod parallel;
pub mod profile;
pub mod provenance_comment;
pub mod query;
mod solver;
pub mod summaries;

pub use boundary::{detect_crate_boundaries, BoundaryViolation, ExternalCall};
pub use ctxt::MigrateCtxt;
pub use greedy::{greedy_resolve, GreedyConfig, GreedyDiagnostic, GreedyPassResult, GreedyStats};
pub use query::{query_function_summary, query_solve_outcome};
pub use solver::{
    build_kir_constraint_graph, graph_fingerprint, outcome_name, solve, solve_with_evidence,
    BoundaryReport, ClusterReport, ConflictReport, ConstraintEdge, ConstraintFactKind,
    ConstraintGraph, ConstraintKind, ConstraintProvenance, PartialReport, SolutionCandidate,
    SolveOutcome, SolverBudget, SolverEvidence,
};

// Legacy exports for backward compatibility
pub use solver::SolveResult;

// ─── Modular pipeline (v0.8.1) ───
// Wires Phases 01-11 into a single `solve_modular` entry point
// that replaces the monolithic `solve()` for the compiler pipeline.

pub use modular_pipeline::{solve_modular, solve_modular_with_evidence, ModularEvidence};
