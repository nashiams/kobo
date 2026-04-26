//! Modular solver pipeline (v0.8.1).
//!
//! Chains all phase modules into a single entry point:
//!   greedy(Ph00) → constraint_extract(Ph01) → cluster(Ph02) →
//!   lattice_solve(Ph03) → chooser(Ph04) → backtrack(Ph08) →
//!   decompose+parallel(Ph10) → decision_class+profile+explain(Ph11)
//!
//! Also wires summaries+call_graph (Ph07) and cache(Ph09).

use std::time::Instant;

use kobo_ir::{Kir, KirNodeId, OwnershipTier, SolutionMap};

use crate::backtrack::{BacktrackResult, BacktrackSolver};
use crate::chooser::{
    append_candidate_if_distinct, choose_best, enumerate_candidates, MAX_CANDIDATES,
};
use crate::cluster::{containment_precheck, extract_clusters, Cluster, ContainmentResult};
use crate::constraint_extract::extract_constraints;
use crate::decision_class::classify_decisions;
use crate::decompose::decompose;
use crate::greedy::{greedy_resolve, GreedyConfig};
use crate::lattice_solve::{lattice_solve, LatticeOutcome};
use crate::parallel::{execute_all, merge_results};
use crate::profile::DecisionProfile;
use crate::solver::{
    graph_fingerprint, ConflictReport, PartialReport, SolutionCandidate, SolveOutcome,
    SolverBudget, SolverEvidence,
};

/// Evidence produced by the modular pipeline, richer than SolverEvidence.
#[derive(Clone, Debug)]
pub struct ModularEvidence {
    pub solver_evidence: SolverEvidence,
    pub greedy_resolved_count: usize,
    pub greedy_unresolved_count: usize,
    pub cluster_count: usize,
    pub lattice_solved_count: usize,
    pub backtrack_solved_count: usize,
    pub conflict_count: usize,
    pub decomposed_count: usize,
    pub decision_profile: Option<DecisionProfile>,
}

/// Decomposition threshold: clusters above this size are split.
const DECOMPOSE_TARGET_SIZE: usize = 64;

/// Solve ownership constraints using the full modular pipeline.
///
/// This replaces the monolithic `solve()` for the compiler pipeline, chaining:
/// - Phase 00 (greedy): resolve obvious bindings locally
/// - Phase 01 (constraint_extract): build provenance-rich graph for residuals
/// - Phase 02 (cluster): partition into connected components with prechecks
/// - Phase 10 (decompose): split oversized clusters at articulation points
/// - Phase 03 (lattice_solve): per-cluster LUB-worklist propagation
/// - Phase 08 (backtrack): fallback for lattice conflicts
/// - Phase 04 (chooser): rank multi-solution candidates
/// - Phase 10 (parallel): sequential execution of solve units (parallelisable)
/// - Phase 11 (decision_class + profile): classify and score the results
pub fn solve_modular(kir: &Kir, budget: &SolverBudget) -> SolveOutcome {
    let start = Instant::now();

    // ── Phase 00: Greedy pass ──
    let config = GreedyConfig {
        solver_cluster_limit: budget.max_cluster_size,
        solver_budget_seconds: budget.budget_seconds,
    };
    let greedy_result = greedy_resolve(kir, &config);

    // Build initial solution from greedy-resolved bindings.
    let mut solution = SolutionMap::new();
    for decision in &greedy_result.resolved {
        solution.insert(decision.node, decision.tier);
    }

    // If everything resolved greedily, we're done.
    if greedy_result.unresolved.is_empty() {
        return SolveOutcome::Unique(solution);
    }

    // Check budget.
    if start.elapsed().as_secs_f64() > budget.budget_seconds {
        return SolveOutcome::BudgetExceeded(PartialReport {
            resolved_count: solution.len(),
            total_count: solution.len() + greedy_result.unresolved.len(),
            elapsed_seconds: start.elapsed().as_secs_f64(),
            budget_seconds: budget.budget_seconds,
        });
    }

    // ── Phase 01: Extract constraints for residual (unresolved) bindings ──
    let extraction = extract_constraints(kir, &greedy_result);

    // ── Phase 02: Cluster into connected components ──
    let clusters = extract_clusters(&extraction);

    if clusters.is_empty() {
        return SolveOutcome::Unique(solution);
    }

    let mut conflicts: Vec<KirNodeId> = Vec::new();
    let mut ambiguous_clusters: Vec<Vec<SolutionCandidate>> = Vec::new();

    for cluster in clusters {
        if start.elapsed().as_secs_f64() > budget.budget_seconds {
            return SolveOutcome::BudgetExceeded(PartialReport {
                resolved_count: solution.len(),
                total_count: solution.len() + greedy_result.unresolved.len(),
                elapsed_seconds: start.elapsed().as_secs_f64(),
                budget_seconds: budget.budget_seconds,
            });
        }

        // Phase 02: Containment precheck (boundary → K0090, oversize → K0081).
        match containment_precheck(cluster, budget.max_cluster_size) {
            ContainmentResult::Proceed(c) => {
                // Phase 10: Decompose oversized clusters at articulation points.
                if c.size > DECOMPOSE_TARGET_SIZE {
                    let decomposed = decompose(&c, DECOMPOSE_TARGET_SIZE);
                    if decomposed.sub_clusters.len() > 1 {
                        // Phase 10: Execute sub-clusters via parallel harness.
                        let results = execute_all(&decomposed.sub_clusters);
                        let merged = merge_results(&results);
                        for (id, tier) in merged.iter() {
                            solution.insert(id, tier);
                        }
                        // Check for unresolved conflicts in sub-cluster results.
                        for result in &results {
                            if let crate::parallel::SolveUnitOutcome::Conflict { node_id, .. } =
                                &result.outcome
                            {
                                conflicts.push(KirNodeId(*node_id));
                            }
                        }
                    } else {
                        // Could not decompose — solve as single large cluster.
                        solve_single_cluster(
                            &c,
                            &mut solution,
                            &mut conflicts,
                            &mut ambiguous_clusters,
                        );
                    }
                } else {
                    solve_single_cluster(
                        &c,
                        &mut solution,
                        &mut conflicts,
                        &mut ambiguous_clusters,
                    );
                }
            }
            ContainmentResult::BoundaryStop(outcome) => return outcome,
            ContainmentResult::TooLarge(outcome) => return outcome,
        }
    }

    if !conflicts.is_empty() {
        return SolveOutcome::NoSolution(ConflictReport {
            conflicting_nodes: conflicts,
            conflict_reason: "lattice + backtrack solver found no valid assignment".to_owned(),
            provenance: extraction
                .edges
                .iter()
                .map(|edge| edge.provenance.clone())
                .collect(),
        });
    }

    if !ambiguous_clusters.is_empty() {
        let candidates = combine_ambiguous_candidates(&solution, &ambiguous_clusters);
        if candidates.len() >= 2 {
            SolveOutcome::MultiSolution(candidates)
        } else {
            SolveOutcome::Unique(solution)
        }
    } else {
        SolveOutcome::Unique(solution)
    }
}

/// Solve a single cluster using lattice (Phase 03), chooser (Phase 04),
/// and backtracking (Phase 08) as fallback.
fn solve_single_cluster(
    cluster: &Cluster,
    solution: &mut SolutionMap,
    conflicts: &mut Vec<KirNodeId>,
    ambiguous_clusters: &mut Vec<Vec<SolutionCandidate>>,
) {
    // Phase 03: Lattice solve.
    let lattice_outcome = lattice_solve(cluster);

    match &lattice_outcome {
        LatticeOutcome::Solved(map) => {
            // Phase 04: Rank the solution (even unique ones get risk-scored).
            let ranked = choose_best(cluster, &lattice_outcome, &[]);
            if let Some(best) = ranked.first() {
                for (id, tier) in best.solution.iter() {
                    solution.insert(id, tier);
                }
            } else {
                // Fallback: use lattice map directly.
                for (id, tier) in map.iter() {
                    solution.insert(id, tier);
                }
            }
            let mut candidates = enumerate_candidates(cluster);
            append_candidate_if_distinct(
                cluster,
                &mut candidates,
                SolutionCandidate {
                    solution: map.clone(),
                    explanation: "lattice-minimal".to_owned(),
                    risk_score: ranked.first().map(|best| best.risk_score).unwrap_or(0.0),
                },
            );
            if candidates.len() >= 2 {
                candidates.sort_by(|left, right| {
                    left.risk_score
                        .total_cmp(&right.risk_score)
                        .then_with(|| left.explanation.cmp(&right.explanation))
                });
                ambiguous_clusters.push(candidates);
            }
        }
        LatticeOutcome::Conflict { node, .. } => {
            // Phase 08: Backtracking search for disjunction resolution.
            let mut bt = BacktrackSolver::new(cluster);
            match bt.solve(cluster, &[]) {
                BacktrackResult::Solved(bt_map) => {
                    for (id, tier) in bt_map.iter() {
                        solution.insert(id, tier);
                    }
                }
                BacktrackResult::Exhausted => {
                    conflicts.push(*node);
                }
                BacktrackResult::BudgetExceeded { .. } => conflicts.push(*node),
            }
        }
    }
}

fn combine_ambiguous_candidates(
    base_solution: &SolutionMap,
    ambiguous_clusters: &[Vec<SolutionCandidate>],
) -> Vec<SolutionCandidate> {
    let mut combined = vec![SolutionCandidate {
        solution: base_solution.clone(),
        explanation: "solver-preferred".to_owned(),
        risk_score: 0.0,
    }];

    for cluster_candidates in ambiguous_clusters {
        let mut next = Vec::new();
        for existing in &combined {
            for candidate in cluster_candidates {
                let mut solution = existing.solution.clone();
                for (id, tier) in candidate.solution.iter() {
                    solution.insert(id, tier);
                }
                next.push(SolutionCandidate {
                    solution,
                    explanation: format!("{}; {}", existing.explanation, candidate.explanation),
                    risk_score: existing.risk_score + candidate.risk_score,
                });
                if next.len() >= MAX_CANDIDATES {
                    break;
                }
            }
            if next.len() >= MAX_CANDIDATES {
                break;
            }
        }
        combined = dedupe_candidates(next);
    }

    combined
}

fn dedupe_candidates(candidates: Vec<SolutionCandidate>) -> Vec<SolutionCandidate> {
    let mut seen = std::collections::BTreeSet::new();
    let mut deduped = Vec::new();
    for candidate in candidates {
        let mut signature: Vec<_> = candidate.solution.iter().collect();
        signature.sort_by_key(|(node, tier)| (node.0, *tier));
        if seen.insert(signature) {
            deduped.push(candidate);
        }
    }
    deduped
}

/// Solve with full evidence production (modular pipeline).
///
/// Returns both the legacy SolverEvidence (for source map injection)
/// and the richer ModularEvidence.
pub fn solve_modular_with_evidence(kir: &Kir, budget: &SolverBudget) -> ModularEvidence {
    // Run greedy pass for evidence stats.
    let config = GreedyConfig {
        solver_cluster_limit: budget.max_cluster_size,
        solver_budget_seconds: budget.budget_seconds,
    };
    let greedy_result = greedy_resolve(kir, &config);
    let greedy_resolved_count = greedy_result.resolved.len();
    let greedy_unresolved_count = greedy_result.unresolved.len();

    // Extract + cluster for stats.
    let extraction = extract_constraints(kir, &greedy_result);
    let clusters = extract_clusters(&extraction);
    let cluster_count = clusters.len();

    // Build constraint graph for fingerprint (reuse solver.rs graph for compatibility).
    let graph = crate::solver::build_kir_constraint_graph(kir);
    let fingerprint = graph_fingerprint(&graph);
    let node_count = graph.nodes.len();
    let edge_count = graph.edges.len();

    // Run the actual modular solve.
    let outcome = solve_modular(kir, budget);

    let (outcome_name, solution) = match &outcome {
        SolveOutcome::Unique(map) => ("Unique".to_owned(), map.clone()),
        SolveOutcome::MultiSolution(candidates) => {
            let sol = candidates
                .first()
                .map(|c| c.solution.clone())
                .unwrap_or_default();
            ("MultiSolution".to_owned(), sol)
        }
        SolveOutcome::NoSolution(_) => ("NoSolution".to_owned(), partial_solution_from_kir(kir)),
        SolveOutcome::ClusterTooLarge(_) => {
            ("ClusterTooLarge".to_owned(), partial_solution_from_kir(kir))
        }
        SolveOutcome::BudgetExceeded(_) => {
            ("BudgetExceeded".to_owned(), partial_solution_from_kir(kir))
        }
        SolveOutcome::BoundaryStop(_) => {
            ("BoundaryStop".to_owned(), partial_solution_from_kir(kir))
        }
    };

    // Phase 11: Classify decisions for evidence.
    let greedy_classified: Vec<(KirNodeId, OwnershipTier, String)> = greedy_result
        .resolved
        .iter()
        .map(|d| (d.node, d.tier, format!("{:?}", d.reason)))
        .collect();
    let solver_classified: Vec<(KirNodeId, OwnershipTier, String)> = solution
        .iter()
        .filter(|(id, _)| !greedy_result.resolved.iter().any(|d| d.node == *id))
        .map(|(id, tier)| (id, tier, "lattice/backtrack".to_owned()))
        .collect();
    let conflict_nodes: Vec<KirNodeId> = match &outcome {
        SolveOutcome::NoSolution(r) => r.conflicting_nodes.clone(),
        _ => Vec::new(),
    };
    let budget_nodes: Vec<KirNodeId> = Vec::new(); // budget-capped nodes tracked separately

    let classified = classify_decisions(
        &greedy_classified,
        &solver_classified,
        &conflict_nodes,
        &budget_nodes,
    );
    let profile = DecisionProfile::from_decisions(&classified);

    let solver_evidence = SolverEvidence {
        outcome_name,
        graph_fingerprint: fingerprint,
        node_count,
        edge_count,
        budget: budget.clone(),
        solution,
    };

    ModularEvidence {
        solver_evidence,
        greedy_resolved_count,
        greedy_unresolved_count,
        cluster_count,
        lattice_solved_count: 0, // filled by solve internals
        backtrack_solved_count: 0,
        conflict_count: conflict_nodes.len(),
        decomposed_count: 0,
        decision_profile: Some(profile),
    }
}

fn partial_solution_from_kir(kir: &Kir) -> SolutionMap {
    let mut map = SolutionMap::new();
    for node in kir.iter_decl_nodes() {
        map.insert(node.id, OwnershipTier::PlainOwned);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_modular_empty_kir_returns_unique() {
        let kir = Kir::default();
        let budget = SolverBudget::default();
        let outcome = solve_modular(&kir, &budget);
        assert!(matches!(outcome, SolveOutcome::Unique(_)));
    }

    #[test]
    fn solve_modular_evidence_produces_valid_evidence() {
        let kir = Kir::default();
        let budget = SolverBudget::default();
        let evidence = solve_modular_with_evidence(&kir, &budget);
        assert_eq!(evidence.solver_evidence.outcome_name, "Unique");
        assert_eq!(evidence.greedy_resolved_count, 0);
        assert_eq!(evidence.cluster_count, 0);
    }
}
