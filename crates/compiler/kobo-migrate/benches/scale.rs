use std::time::{Duration, Instant};

use kobo_ir::{KirNodeId, OwnershipTier};
use kobo_migrate::cluster::{Cluster, ClusterId};
use kobo_migrate::constraint_extract::ConstraintNode;
use kobo_migrate::parallel::{execute_all, merge_results};

fn main() {
    let clusters = build_clusters(120, 128);
    let expected_nodes = 120 * 128;

    let cold_started = Instant::now();
    let cold_results = execute_all(&clusters);
    let cold_elapsed = cold_started.elapsed();
    let cold_map = merge_results(&cold_results);
    assert_eq!(cold_map.len(), expected_nodes);

    let warm_started = Instant::now();
    let warm_results = execute_all(&clusters);
    let warm_elapsed = warm_started.elapsed();
    let warm_map = merge_results(&warm_results);
    assert_eq!(warm_map.len(), expected_nodes);

    assert!(
        cold_elapsed < Duration::from_secs(10),
        "cold 15k-node parallel solve exceeded production sanity budget: {cold_elapsed:?}"
    );
    assert!(
        warm_elapsed < Duration::from_secs(10),
        "warm 15k-node parallel solve exceeded production sanity budget: {warm_elapsed:?}"
    );

    println!("scale: nodes={expected_nodes} cold={cold_elapsed:?} warm={warm_elapsed:?}");
}

fn build_clusters(cluster_count: u32, nodes_per_cluster: u32) -> Vec<Cluster> {
    (0..cluster_count)
        .map(|cluster_id| make_cluster(cluster_id, nodes_per_cluster))
        .collect()
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
