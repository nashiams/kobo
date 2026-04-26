//! Phase 09: Query layer over MigrateCtxt.
//!
//! Provides semantic queries that phases can call instead of raw field access.
//! Queries go through the cache when appropriate.

use kobo_ir::SolutionMap;

use crate::cluster::{extract_clusters, Cluster};
use crate::constraint_extract::extract_constraints;
use crate::ctxt::MigrateCtxt;
use crate::greedy::greedy_resolve;
use crate::lattice_solve::{lattice_solve, LatticeOutcome};
use crate::summaries::FunctionSummary;

/// Query: resolve ownership for all bindings.
///
/// Runs greedy → extract → cluster → lattice pipeline, caching results.
pub fn query_solve_all(ctxt: &mut MigrateCtxt) -> SolutionMap {
    // Check cache first.
    if let Some(cached) = ctxt.cache().get_solution() {
        return cached.clone();
    }

    // Greedy pass.
    let greedy_result = greedy_resolve(&ctxt.kir, &ctxt.config);

    // Build initial solution from greedy resolved.
    let mut solution = SolutionMap::new();
    for decision in &greedy_result.resolved {
        solution.insert(decision.node, decision.tier);
    }

    // Extract constraints for residuals.
    let extraction = extract_constraints(&ctxt.kir, &greedy_result);

    // Cluster.
    let clusters = extract_clusters(&extraction);

    // Lattice-solve each cluster.
    for cluster in &clusters {
        match lattice_solve(cluster) {
            LatticeOutcome::Solved(cluster_map) => {
                for (id, tier) in cluster_map.iter() {
                    solution.insert(id, tier);
                }
            }
            LatticeOutcome::Conflict { node, floor, .. } => {
                // Conflict: fall back to floor.
                solution.insert(node, floor);
            }
        }
    }

    // Cache the result.
    ctxt.cache_mut().set_solution(solution.clone());

    solution
}

/// Query: get function summary by name.
pub fn query_function_summary<'a>(
    ctxt: &'a MigrateCtxt,
    name: &str,
) -> Option<&'a FunctionSummary> {
    ctxt.summaries().get(name)
}

/// Query: get clusters for the current KIR.
pub fn query_clusters(ctxt: &MigrateCtxt) -> Vec<Cluster> {
    let greedy_result = greedy_resolve(&ctxt.kir, &ctxt.config);
    let extraction = extract_constraints(&ctxt.kir, &greedy_result);
    extract_clusters(&extraction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::greedy::GreedyConfig;

    #[test]
    fn query_solve_all_caches_result() {
        let kir = kobo_ir::Kir::default();
        let config = GreedyConfig {
            solver_cluster_limit: 256,
            solver_budget_seconds: 5.0,
        };
        let mut ctxt = MigrateCtxt::new(kir, config);

        let r1 = query_solve_all(&mut ctxt);
        let r2 = query_solve_all(&mut ctxt);
        assert_eq!(r1.len(), r2.len());
    }
}
