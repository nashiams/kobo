//! Stage: Candidate ranking and selection.
//!
//! When the solver has multiple valid solutions, rank them by risk score
//! and heuristics.  Caps enumeration at 8 candidates to keep output manageable.

use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap};

use crate::cluster::Cluster;
use crate::constraint_extract::ConstraintNode;
use crate::lattice_solve::LatticeOutcome;
use crate::solver::{ConstraintKind, SolutionCandidate};

/// Maximum candidates to enumerate.
pub const MAX_CANDIDATES: usize = 8;

/// Ranked candidate with computed metrics.
#[derive(Clone, Debug)]
pub struct RankedCandidate {
    pub solution: SolutionMap,
    pub risk_score: f64,
    pub explanation: String,
    pub rank: usize,
}

/// Choose the best ownership solution from a set of candidates.
///
/// If the lattice solver yields a unique solution, use it directly.
/// Otherwise, enumerate alternatives up to `MAX_CANDIDATES` and pick the
/// lowest-risk one.
pub fn choose_best(
    cluster: &Cluster,
    lattice_outcome: &LatticeOutcome,
    candidates: &[SolutionCandidate],
) -> Vec<RankedCandidate> {
    match lattice_outcome {
        LatticeOutcome::Solved(map) => {
            // Single lattice solution — rank as #1 with risk from heuristics.
            let risk = compute_risk(map, &cluster.nodes);
            vec![RankedCandidate {
                solution: map.clone(),
                risk_score: risk,
                explanation: "lattice-unique".to_owned(),
                rank: 1,
            }]
        }
        LatticeOutcome::Conflict { .. } | LatticeOutcome::IterationBudgetExceeded { .. } => {
            // Lattice conflict or iteration budget exceeded — fall through to solver candidates.
            rank_solver_candidates(candidates, &cluster.nodes)
        }
    }
}

/// Rank a set of solver candidates by risk score.
fn rank_solver_candidates(
    candidates: &[SolutionCandidate],
    _nodes: &[ConstraintNode],
) -> Vec<RankedCandidate> {
    let mut ranked: Vec<RankedCandidate> = candidates
        .iter()
        .take(MAX_CANDIDATES)
        .map(|c| {
            let risk = c.risk_score;
            RankedCandidate {
                solution: c.solution.clone(),
                risk_score: risk,
                explanation: c.explanation.clone(),
                rank: 0,
            }
        })
        .collect();

    // Re-rank by ascending risk (lower = better).
    ranked.sort_by(|a, b| {
        a.risk_score
            .total_cmp(&b.risk_score)
            .then_with(|| a.explanation.cmp(&b.explanation))
    });
    for (i, r) in ranked.iter_mut().enumerate() {
        r.rank = i + 1;
    }

    ranked
}

/// Compute a heuristic risk score for a solution.
///
/// Lower is better.
/// - Each ArcMutShared adds +2.0 (deadlock risk).
/// - Each ArcShared adds +1.0 (overhead).
/// - Each RcMutShared adds +1.5 (panic risk on double borrow).
/// - Each RcShared adds +0.5 (light overhead).
/// - Violations of ceiling add +5.0 per node.
fn compute_risk(map: &SolutionMap, nodes: &[ConstraintNode]) -> f64 {
    let mut score: f64 = 0.0;
    for (id, tier) in map.iter() {
        score += tier_penalty(tier);

        // Check ceiling violations.
        if let Some(node) = nodes.iter().find(|n| n.id == id) {
            if let Some(ceil) = node.ceiling {
                if crate::lattice_solve::tier_rank(tier) > crate::lattice_solve::tier_rank(ceil) {
                    score += 5.0;
                }
            }
        }
    }
    score
}

fn tier_penalty(tier: OwnershipTier) -> f64 {
    match tier {
        OwnershipTier::PlainOwned | OwnershipTier::BoxOwned => 0.0,
        OwnershipTier::RcShared => 0.5,
        OwnershipTier::ArcShared => 1.0,
        OwnershipTier::RcMutShared => 1.5,
        OwnershipTier::ArcMutShared => 2.0,
        _ => 0.0,
    }
}

/// Enumerate candidate solutions for a cluster by varying each node's tier.
///
/// Capped at `MAX_CANDIDATES` to prevent combinatorial explosion.
pub fn enumerate_candidates(cluster: &Cluster) -> Vec<SolutionCandidate> {
    let mut candidates = Vec::new();

    // Strategy: start from floors, then try lifting each node by one tier.
    let base: SolutionMap = {
        let mut m = SolutionMap::new();
        for node in &cluster.nodes {
            m.insert(node.id, node.floor);
        }
        m
    };
    push_candidate_if_valid(
        cluster,
        &mut candidates,
        SolutionCandidate {
            solution: base.clone(),
            explanation: "all-at-floor".to_owned(),
            risk_score: compute_risk(&base, &cluster.nodes),
        },
    );

    // For each node, try the next tier above floor (if within ceiling).
    for node in &cluster.nodes {
        if candidates.len() >= MAX_CANDIDATES {
            break;
        }
        if let Some(next_tier) = next_tier(node.floor) {
            let in_ceiling = node
                .ceiling
                .map(|c| {
                    crate::lattice_solve::tier_rank(next_tier) <= crate::lattice_solve::tier_rank(c)
                })
                .unwrap_or(true);
            if in_ceiling {
                let mut variant = base.clone();
                variant.insert(node.id, next_tier);
                let risk = compute_risk(&variant, &cluster.nodes);
                push_candidate_if_valid(
                    cluster,
                    &mut candidates,
                    SolutionCandidate {
                        solution: variant,
                        explanation: format!("{} lifted to {:?}", node.binding_name, next_tier),
                        risk_score: risk,
                    },
                );
            }
        }
    }

    candidates
}

pub fn append_candidate_if_distinct(
    cluster: &Cluster,
    candidates: &mut Vec<SolutionCandidate>,
    candidate: SolutionCandidate,
) {
    push_candidate_if_valid(cluster, candidates, candidate);
}

fn push_candidate_if_valid(
    cluster: &Cluster,
    candidates: &mut Vec<SolutionCandidate>,
    candidate: SolutionCandidate,
) {
    if candidates.len() >= MAX_CANDIDATES
        || !candidate_satisfies_cluster(cluster, &candidate.solution)
    {
        return;
    }

    let duplicate = candidates.iter().any(|existing| {
        solution_signature(&existing.solution) == solution_signature(&candidate.solution)
    });
    if !duplicate {
        candidates.push(candidate);
    }
}

pub fn candidate_satisfies_cluster(cluster: &Cluster, solution: &SolutionMap) -> bool {
    cluster.raw_edges.iter().all(|edge| {
        let source = match solution.get(edge.source) {
            Some(tier) => tier,
            None => return false,
        };
        let target = match solution.get(edge.target) {
            Some(tier) => tier,
            None => return false,
        };
        match edge.kind {
            ConstraintKind::PropagateSharing => !source.is_shared() || target.is_shared(),
            ConstraintKind::PropagateSend => !source.is_thread_safe() || target.is_thread_safe(),
            ConstraintKind::MutuallyExclusive => {
                !(is_mutably_shared(source) && is_mutably_shared(target))
            }
        }
    })
}

fn solution_signature(solution: &SolutionMap) -> Vec<(KirNodeId, OwnershipTier)> {
    let mut signature: Vec<_> = solution.iter().collect();
    signature.sort_by_key(|(node, tier)| (node.0, *tier));
    signature
}

fn is_mutably_shared(tier: OwnershipTier) -> bool {
    matches!(
        tier,
        OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared
    )
}

fn next_tier(tier: OwnershipTier) -> Option<OwnershipTier> {
    match tier {
        OwnershipTier::PlainOwned => Some(OwnershipTier::BoxOwned),
        OwnershipTier::BoxOwned => Some(OwnershipTier::RcShared),
        OwnershipTier::RcShared => Some(OwnershipTier::ArcShared),
        OwnershipTier::ArcShared => Some(OwnershipTier::RcMutShared),
        OwnershipTier::RcMutShared => Some(OwnershipTier::ArcMutShared),
        OwnershipTier::ArcMutShared => Some(OwnershipTier::Scoped),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterId};
    use crate::constraint_extract::ConstraintNode;
    use kobo_ir::KirNodeId;

    fn make_cluster(n: usize) -> Cluster {
        let nodes: Vec<ConstraintNode> = (0..n)
            .map(|i| ConstraintNode {
                id: KirNodeId(i as u32),
                floor: OwnershipTier::PlainOwned,
                ceiling: None,
                is_boundary: false,
                binding_name: format!("v{}", i),
            })
            .collect();
        let size = nodes.len();
        Cluster {
            id: ClusterId(0),
            nodes,
            edges: Vec::new(),
            raw_edges: Vec::new(),
            size,
        }
    }

    #[test]
    fn enumerate_caps_at_max() {
        let cluster = make_cluster(20); // 20 nodes would produce 21 candidates
        let candidates = enumerate_candidates(&cluster);
        assert!(candidates.len() <= MAX_CANDIDATES);
    }

    #[test]
    fn floor_solution_has_lowest_risk() {
        let cluster = make_cluster(3);
        let candidates = enumerate_candidates(&cluster);
        let min_risk = candidates
            .iter()
            .map(|c| c.risk_score)
            .fold(f64::INFINITY, f64::min);
        assert_eq!(candidates[0].risk_score, min_risk);
    }

    #[test]
    fn ranked_candidates_sorted_ascending() {
        let cluster = make_cluster(3);
        let lattice_outcome = crate::lattice_solve::LatticeOutcome::Conflict {
            node: KirNodeId(0),
            floor: OwnershipTier::ArcShared,
            ceiling: OwnershipTier::RcShared,
        };
        let candidates = enumerate_candidates(&cluster);
        let ranked = choose_best(&cluster, &lattice_outcome, &candidates);
        for w in ranked.windows(2) {
            assert!(w[0].risk_score <= w[1].risk_score);
        }
    }
}
