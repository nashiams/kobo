use std::path::PathBuf;

use kobo_ir::{Kir, KirNodeId, OwnershipTier, SolutionMap};
use kobo_migrate::cluster::{Cluster, ClusterId};
use kobo_migrate::constraint_extract::ConstraintNode;
use kobo_migrate::parallel::{execute_all, merge_results, SolveUnitOutcome};
use kobo_migrate::{query_solve_outcome, GreedyConfig, MigrateCtxt, SolveOutcome, SolverBudget};

fn stress_cache_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "kobo-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn make_cluster(cluster_id: u32, node_count: u32) -> Cluster {
    let nodes: Vec<_> = (0..node_count)
        .map(|offset| ConstraintNode {
            id: KirNodeId(cluster_id * 10_000 + offset),
            floor: if offset % 3 == 0 {
                OwnershipTier::ArcShared
            } else {
                OwnershipTier::PlainOwned
            },
            ceiling: None,
            is_boundary: false,
            binding_name: format!("c{cluster_id}_v{offset}"),
        })
        .collect();

    Cluster {
        id: ClusterId(cluster_id),
        size: nodes.len(),
        nodes,
        edges: Vec::new(),
        raw_edges: Vec::new(),
    }
}

#[test]
fn parallel_solver_handles_many_independent_clusters_without_losing_nodes() {
    let cluster_count = 128u32;
    let nodes_per_cluster = 16u32;
    let clusters: Vec<_> = (0..cluster_count)
        .map(|cluster_id| make_cluster(cluster_id, nodes_per_cluster))
        .collect();

    let results = execute_all(&clusters);

    assert_eq!(results.len(), cluster_count as usize);
    for (expected_id, result) in results.iter().enumerate() {
        assert_eq!(result.cluster_id, expected_id as u32);
        assert!(
            matches!(result.outcome, SolveUnitOutcome::Solved(_)),
            "cluster {} did not solve: {:?}",
            result.cluster_id,
            result.outcome
        );
    }

    let merged = merge_results(&results);
    assert_eq!(merged.len(), (cluster_count * nodes_per_cluster) as usize);
    assert_eq!(
        merged.get(KirNodeId(0)),
        Some(OwnershipTier::ArcShared),
        "first floor assignment must survive merge"
    );
    assert_eq!(
        merged.get(KirNodeId(1)),
        Some(OwnershipTier::PlainOwned),
        "plain assignment must survive merge"
    );
}

#[test]
fn query_solve_outcome_reuses_disk_cache_for_matching_graph_fingerprint() {
    let cache_dir = stress_cache_dir("query-cache");
    let config = GreedyConfig {
        solver_cluster_limit: 256,
        solver_budget_seconds: 5.0,
        mutable_sites_threshold: GreedyConfig::default().mutable_sites_threshold,
    };

    let mut first = MigrateCtxt::new_with_cache_dir(Kir::default(), config.clone(), &cache_dir);
    let first_outcome = query_solve_outcome(&mut first, &SolverBudget::default());
    assert!(matches!(first_outcome, SolveOutcome::Unique(_)));

    let cache_entries: Vec<_> = std::fs::read_dir(&cache_dir)
        .expect("cache directory should exist after first query")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "solution")
        })
        .collect();
    assert_eq!(
        cache_entries.len(),
        1,
        "first query must persist exactly one fingerprinted cache entry"
    );

    let mut second = MigrateCtxt::new_with_cache_dir(Kir::default(), config, &cache_dir);
    let before_generation = second.cache().generation();
    let second_outcome = query_solve_outcome(&mut second, &SolverBudget::default());
    assert!(matches!(second_outcome, SolveOutcome::Unique(_)));
    assert!(
        second.cache().generation() > before_generation,
        "loading a disk outcome should populate the in-memory cache"
    );

    let _ = std::fs::remove_dir_all(cache_dir);
}

#[test]
fn unique_outcome_cache_does_not_store_non_unique_results() {
    let cache_dir = stress_cache_dir("non-unique-cache");
    let map = SolutionMap::new();
    let outcome = SolveOutcome::MultiSolution(vec![kobo_migrate::SolutionCandidate {
        solution: map,
        explanation: "stress candidate".to_owned(),
        risk_score: 0.0,
    }]);

    let wrote =
        kobo_migrate::cache::SolverCache::save_unique_outcome(&cache_dir, "nonunique", &outcome)
            .expect("non-unique cache save should not fail");

    assert!(!wrote, "non-unique outcomes must be rebuilt, not cached");
    assert!(
        !cache_dir.exists(),
        "non-unique cache attempt should not create durable state"
    );
}
