//! Stage: Parallel solving infrastructure.
//!
//! Provides a unit-of-computation harness for solving clusters.
//! Each cluster solve is a self-contained unit that can run independently.

use kobo_ir::SolutionMap;

use crate::cluster::Cluster;
use crate::lattice_solve::{lattice_solve, LatticeOutcome};

/// A unit of computation: one cluster solve.
#[derive(Clone, Debug)]
pub struct SolveUnit {
    pub cluster_id: u32,
    pub node_count: usize,
    pub edge_count: usize,
}

/// Result of one solve unit.
#[derive(Clone, Debug)]
pub struct SolveUnitResult {
    pub cluster_id: u32,
    pub outcome: SolveUnitOutcome,
}

/// Outcome variants for a solve unit.
#[derive(Clone, Debug)]
pub enum SolveUnitOutcome {
    Solved(SolutionMap),
    Conflict { node_id: u32, reason: String },
}

/// Plan a batch of solve units from clusters.
pub fn plan_solve_units(clusters: &[Cluster]) -> Vec<SolveUnit> {
    clusters
        .iter()
        .map(|c| SolveUnit {
            cluster_id: c.id.0,
            node_count: c.size,
            edge_count: c.raw_edges.len(),
        })
        .collect()
}

/// Execute a single solve unit (cluster → lattice solution).
///
/// This is a pure function — no side effects — suitable for parallel dispatch.
pub fn execute_solve_unit(cluster: &Cluster) -> SolveUnitResult {
    let outcome = lattice_solve(cluster);
    match outcome {
        LatticeOutcome::Solved(map) => SolveUnitResult {
            cluster_id: cluster.id.0,
            outcome: SolveUnitOutcome::Solved(map),
        },
        LatticeOutcome::Conflict {
            node,
            floor,
            ceiling,
        } => SolveUnitResult {
            cluster_id: cluster.id.0,
            outcome: SolveUnitOutcome::Conflict {
                node_id: node.0,
                reason: format!("floor {:?} exceeds ceiling {:?}", floor, ceiling),
            },
        },
        LatticeOutcome::IterationBudgetExceeded { iterations, .. } => SolveUnitResult {
            cluster_id: cluster.id.0,
            outcome: SolveUnitOutcome::Conflict {
                node_id: 0,
                reason: format!("iteration budget exceeded after {} iterations", iterations),
            },
        },
    }
}

/// Execute all solve units in parallel and preserve input order in the result.
pub fn execute_all(clusters: &[Cluster]) -> Vec<SolveUnitResult> {
    if clusters.len() <= 1 {
        return clusters.iter().map(execute_solve_unit).collect();
    }

    std::thread::scope(|scope| {
        let handles: Vec<_> = clusters
            .iter()
            .map(|cluster| {
                let cluster_id = cluster.id.0;
                (cluster_id, scope.spawn(move || execute_solve_unit(cluster)))
            })
            .collect();

        handles
            .into_iter()
            .map(|(cluster_id, handle)| match handle.join() {
                Ok(result) => result,
                Err(_) => SolveUnitResult {
                    cluster_id,
                    outcome: SolveUnitOutcome::Conflict {
                        node_id: 0,
                        reason: "parallel solve worker panicked".to_owned(),
                    },
                },
            })
            .collect()
    })
}

/// Merge solve unit results into a single solution map.
pub fn merge_results(results: &[SolveUnitResult]) -> SolutionMap {
    let mut merged = SolutionMap::new();
    for result in results {
        if let SolveUnitOutcome::Solved(ref map) = result.outcome {
            for (id, tier) in map.iter() {
                merged.insert(id, tier);
            }
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterId};
    use crate::constraint_extract::ConstraintNode;
    use kobo_ir::{KirNodeId, OwnershipTier};

    fn make_cluster(id: u32, n: usize) -> Cluster {
        let nodes: Vec<ConstraintNode> = (0..n)
            .map(|i| ConstraintNode {
                id: KirNodeId(id * 100 + i as u32),
                floor: OwnershipTier::PlainOwned,
                ceiling: None,
                is_boundary: false,
                binding_name: format!("c{}_v{}", id, i),
            })
            .collect();
        let size = nodes.len();
        Cluster {
            id: ClusterId(id),
            nodes,
            edges: Vec::new(),
            raw_edges: Vec::new(),
            size,
        }
    }

    #[test]
    fn plan_produces_one_unit_per_cluster() {
        let clusters = vec![make_cluster(0, 3), make_cluster(1, 5)];
        let units = plan_solve_units(&clusters);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].node_count, 3);
        assert_eq!(units[1].node_count, 5);
    }

    #[test]
    fn single_node_cluster_solves() {
        let cluster = make_cluster(0, 1);
        let result = execute_solve_unit(&cluster);
        assert!(matches!(result.outcome, SolveUnitOutcome::Solved(_)));
    }

    #[test]
    fn merge_combines_solutions() {
        let clusters = vec![make_cluster(0, 2), make_cluster(1, 3)];
        let results = execute_all(&clusters);
        let merged = merge_results(&results);
        assert_eq!(merged.len(), 5);
    }

    #[test]
    fn execute_all_preserves_cluster_order_under_parallel_dispatch() {
        let clusters: Vec<_> = (0..32).map(|id| make_cluster(id, 4)).collect();
        let results = execute_all(&clusters);
        assert_eq!(results.len(), clusters.len());
        for (idx, result) in results.iter().enumerate() {
            assert_eq!(result.cluster_id, idx as u32);
            assert!(matches!(result.outcome, SolveUnitOutcome::Solved(_)));
        }
    }
}
