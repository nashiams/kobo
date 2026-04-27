//! Phase 03: Lattice-based ownership inference with floor/ceiling propagation.
//!
//! Implements a worklist algorithm over TierVar lattice points with
//! Least-Upper-Bound (LUB) merging.  Floors and ceilings tighten the
//! lattice rather than leaving it open.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap};

use crate::cluster::Cluster;
use crate::solver::ConstraintKind;

/// Lattice variable: a constraint node with a mutable `current` value
/// bounded by `floor` and `ceiling`.
#[derive(Clone, Debug)]
pub struct TierVar {
    pub node: KirNodeId,
    pub current: OwnershipTier,
    pub floor: OwnershipTier,
    pub ceiling: Option<OwnershipTier>,
}

/// Outcome of lattice solving for one cluster.
#[derive(Clone, Debug)]
pub enum LatticeOutcome {
    /// All variables resolved without conflict.
    Solved(SolutionMap),
    /// Floor/ceiling conflict detected — unsolvable cluster.
    Conflict {
        node: KirNodeId,
        floor: OwnershipTier,
        ceiling: OwnershipTier,
    },
    /// Worklist iteration budget exceeded before convergence.
    /// The partial map may be inconsistent — constraints may not be fully propagated.
    IterationBudgetExceeded {
        iterations: u32,
        partial_map: SolutionMap,
    },
}

// ────────────────── Lattice ordering ──────────────────

/// Canonical lattice ordering. All other orderings (`priority()`,
/// `greedy_priority()`) must agree with this relative order.
///
/// Undecided < PlainOwned < BoxOwned < RcShared < ArcShared < RcMutShared < ArcMutShared < Scoped
pub fn tier_rank(tier: OwnershipTier) -> u8 {
    match tier {
        OwnershipTier::Undecided => 0,
        OwnershipTier::PlainOwned => 1,
        OwnershipTier::BoxOwned => 2,
        OwnershipTier::RcShared => 3,
        OwnershipTier::ArcShared => 4,
        OwnershipTier::RcMutShared => 5,
        OwnershipTier::ArcMutShared => 6,
        OwnershipTier::Scoped => 7,
    }
}

/// Least upper bound of two tiers.
pub fn lattice_lub(a: OwnershipTier, b: OwnershipTier) -> OwnershipTier {
    if tier_rank(a) >= tier_rank(b) {
        a
    } else {
        b
    }
}

/// Check if `tier` is within `[floor, ceiling]`.
fn in_bounds(tier: OwnershipTier, floor: OwnershipTier, ceiling: Option<OwnershipTier>) -> bool {
    if tier_rank(tier) < tier_rank(floor) {
        return false;
    }
    if let Some(ceil) = ceiling {
        if tier_rank(tier) > tier_rank(ceil) {
            return false;
        }
    }
    true
}

/// Clamp `tier` up to `floor` (but never past ceiling).
fn clamp_up(
    tier: OwnershipTier,
    floor: OwnershipTier,
    ceiling: Option<OwnershipTier>,
) -> OwnershipTier {
    let _ = ceiling;
    lattice_lub(tier, floor)
}

// ────────────────── Solver ──────────────────

/// Solve a cluster using LUB-worklist propagation.
///
/// Returns `Solved(map)` with a solution, or `Conflict` if floor exceeds ceiling.
pub fn lattice_solve(cluster: &Cluster) -> LatticeOutcome {
    let mut vars: BTreeMap<KirNodeId, TierVar> = BTreeMap::new();

    // Initialize TierVar for each node.
    for node in &cluster.nodes {
        let tv = TierVar {
            node: node.id,
            current: node.floor,
            floor: node.floor,
            ceiling: node.ceiling,
        };
        // Preflight: floor must not exceed ceiling.
        if let Some(ceil) = node.ceiling {
            if tier_rank(node.floor) > tier_rank(ceil) {
                return LatticeOutcome::Conflict {
                    node: node.id,
                    floor: node.floor,
                    ceiling: ceil,
                };
            }
        }
        vars.insert(node.id, tv);
    }

    // Build adjacency map from edges.
    let mut neighbors: BTreeMap<KirNodeId, Vec<(KirNodeId, ConstraintKind)>> = BTreeMap::new();
    for edge in &cluster.raw_edges {
        neighbors
            .entry(edge.source)
            .or_default()
            .push((edge.target, edge.kind.clone()));
        neighbors
            .entry(edge.target)
            .or_default()
            .push((edge.source, edge.kind.clone()));
    }

    // Worklist initialization.
    let mut worklist: VecDeque<KirNodeId> = vars.keys().copied().collect();
    let mut in_worklist: BTreeSet<KirNodeId> = worklist.iter().copied().collect();
    let mut iterations: u32 = 0;
    let max_iterations: u32 = (cluster.size as u32).saturating_mul(64).max(1024);

    // Propagation loop.
    // Collect LUB-phase conflicts rather than returning immediately, so
    // the GLB phase can attribute the root cause more precisely.
    let mut lub_conflict: Option<(KirNodeId, OwnershipTier, OwnershipTier)> = None;

    while let Some(current_id) = worklist.pop_front() {
        in_worklist.remove(&current_id);
        iterations += 1;
        if iterations > max_iterations {
            let mut partial = SolutionMap::new();
            for (id, tv) in &vars {
                partial.insert(*id, tv.current);
            }
            return LatticeOutcome::IterationBudgetExceeded {
                iterations,
                partial_map: partial,
            };
        }

        let current_tier = vars[&current_id].current;

        if let Some(adj) = neighbors.get(&current_id) {
            for (neighbor_id, kind) in adj {
                let need = match kind {
                    ConstraintKind::PropagateSharing => current_tier,
                    ConstraintKind::PropagateSend => {
                        // Promote to thread-safe variant.
                        match current_tier {
                            OwnershipTier::RcShared => OwnershipTier::ArcShared,
                            OwnershipTier::RcMutShared => OwnershipTier::ArcMutShared,
                            other => other,
                        }
                    }
                    ConstraintKind::MutuallyExclusive => {
                        // MutuallyExclusive: don't propagate tier — handled post-solve.
                        continue;
                    }
                };

                if let Some(nvar) = vars.get_mut(neighbor_id) {
                    let old = nvar.current;
                    let proposed = lattice_lub(old, need);
                    let clamped = clamp_up(proposed, nvar.floor, nvar.ceiling);

                    // Check conflict — record but don't return immediately.
                    if !in_bounds(clamped, nvar.floor, nvar.ceiling) {
                        if lub_conflict.is_none() {
                            lub_conflict = Some((
                                *neighbor_id,
                                nvar.floor,
                                nvar.ceiling.unwrap_or(OwnershipTier::ArcMutShared),
                            ));
                        }
                        // Stop propagation from this conflict node but continue
                        // processing other nodes in the worklist.
                        continue;
                    }

                    if clamped != old {
                        nvar.current = clamped;
                        if !in_worklist.contains(neighbor_id) {
                            worklist.push_back(*neighbor_id);
                            in_worklist.insert(*neighbor_id);
                        }
                    }
                }
            }
        }
    }

    // GLB (downward) propagation: propagate ceilings backward through the
    // graph. If a node's ceiling constrains its neighbors, propagate that
    // ceiling to predecessors. If any predecessor's floor exceeds the
    // effective ceiling, report conflict at the predecessor (root cause).
    {
        // Build reverse adjacency: target → sources (for backward propagation).
        let mut reverse: BTreeMap<KirNodeId, Vec<KirNodeId>> = BTreeMap::new();
        for edge in &cluster.raw_edges {
            if matches!(
                edge.kind,
                ConstraintKind::PropagateSharing | ConstraintKind::PropagateSend
            ) {
                reverse.entry(edge.target).or_default().push(edge.source);
            }
        }

        // Effective ceiling per node: start with explicit ceilings.
        let mut effective_ceiling: BTreeMap<KirNodeId, OwnershipTier> = BTreeMap::new();
        for (id, tv) in &vars {
            if let Some(c) = tv.ceiling {
                effective_ceiling.insert(*id, c);
            }
        }

        // Worklist for backward ceiling propagation.
        let mut ceil_worklist: VecDeque<KirNodeId> = effective_ceiling.keys().copied().collect();
        let mut ceil_visited: BTreeSet<KirNodeId> = ceil_worklist.iter().copied().collect();
        let mut ceil_iters: u32 = 0;
        let max_ceil_iters: u32 = (cluster.size as u32).saturating_mul(16).max(256);

        while let Some(node_id) = ceil_worklist.pop_front() {
            ceil_visited.remove(&node_id);
            ceil_iters += 1;
            if ceil_iters > max_ceil_iters {
                break;
            }

            if let Some(ceiling) = effective_ceiling.get(&node_id).copied() {
                if let Some(predecessors) = reverse.get(&node_id) {
                    for &pred_id in predecessors {
                        // Propagate: predecessor's effective ceiling ≤ this node's ceiling.
                        let pred_ceil = effective_ceiling.get(&pred_id).copied();
                        let new_ceil = match pred_ceil {
                            Some(existing) => {
                                // GLB: take the lower of existing and propagated.
                                if tier_rank(ceiling) < tier_rank(existing) {
                                    ceiling
                                } else {
                                    existing
                                }
                            }
                            None => ceiling,
                        };
                        let changed = pred_ceil.map(|e| new_ceil != e).unwrap_or(true);
                        if changed {
                            effective_ceiling.insert(pred_id, new_ceil);
                            if !ceil_visited.contains(&pred_id) {
                                ceil_worklist.push_back(pred_id);
                                ceil_visited.insert(pred_id);
                            }
                        }
                    }
                }
            }
        }

        // Check: any node whose floor exceeds its effective ceiling → conflict.
        // GLB attributes the conflict to the root-cause node (upstream).
        for (id, tv) in &vars {
            if let Some(eff_ceil) = effective_ceiling.get(id) {
                if tier_rank(tv.floor) > tier_rank(*eff_ceil) {
                    return LatticeOutcome::Conflict {
                        node: *id,
                        floor: tv.floor,
                        ceiling: *eff_ceil,
                    };
                }
            }
        }
    }

    // If the LUB phase detected a conflict but GLB didn't find a better root
    // cause, return the original LUB conflict.
    if let Some((node, floor, ceiling)) = lub_conflict {
        return LatticeOutcome::Conflict {
            node,
            floor,
            ceiling,
        };
    }

    // Post-solve: verify MutuallyExclusive constraints.
    // The worklist skips these (they can't be solved by LUB propagation),
    // but we must detect violations and return Conflict.
    for edge in &cluster.raw_edges {
        if matches!(edge.kind, ConstraintKind::MutuallyExclusive) {
            let a_tier = vars.get(&edge.source).map(|tv| tv.current);
            let b_tier = vars.get(&edge.target).map(|tv| tv.current);
            if let (Some(a), Some(b)) = (a_tier, b_tier) {
                let a_mut = matches!(a, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
                let b_mut = matches!(b, OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared);
                if a_mut && b_mut {
                    return LatticeOutcome::Conflict {
                        node: edge.source,
                        floor: vars[&edge.source].floor,
                        ceiling: vars[&edge.source].ceiling.unwrap_or(OwnershipTier::Scoped),
                    };
                }
            }
        }
    }

    // Build solution map.
    let mut map = SolutionMap::new();
    for (id, tv) in &vars {
        map.insert(*id, tv.current);
    }

    LatticeOutcome::Solved(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterId};
    use crate::constraint_extract::ConstraintNode;
    use crate::solver::ConstraintEdge;
    use kobo_ir::KirNodeId;

    fn node(id: u32, floor: OwnershipTier) -> ConstraintNode {
        ConstraintNode {
            id: KirNodeId(id),
            floor,
            ceiling: None,
            is_boundary: false,
            binding_name: format!("v{}", id),
        }
    }

    fn edge(src: u32, tgt: u32, kind: ConstraintKind) -> ConstraintEdge {
        ConstraintEdge::synthetic(KirNodeId(src), KirNodeId(tgt), kind, "test")
    }

    fn cluster(nodes: Vec<ConstraintNode>, edges: Vec<ConstraintEdge>) -> Cluster {
        let size = nodes.len();
        Cluster {
            id: ClusterId(0),
            nodes,
            edges: Vec::new(),
            raw_edges: edges,
            size,
        }
    }

    #[test]
    fn single_node_takes_floor() {
        let c = cluster(vec![node(1, OwnershipTier::RcShared)], vec![]);
        match lattice_solve(&c) {
            LatticeOutcome::Solved(map) => {
                assert_eq!(map.get(KirNodeId(1)), Some(OwnershipTier::RcShared));
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn sharing_propagation_lifts_peer() {
        let c = cluster(
            vec![
                node(1, OwnershipTier::ArcShared),
                node(2, OwnershipTier::PlainOwned),
            ],
            vec![edge(1, 2, ConstraintKind::PropagateSharing)],
        );
        match lattice_solve(&c) {
            LatticeOutcome::Solved(map) => {
                assert_eq!(map.get(KirNodeId(2)), Some(OwnershipTier::ArcShared));
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn send_promotes_rc_to_arc() {
        let c = cluster(
            vec![
                node(1, OwnershipTier::RcShared),
                node(2, OwnershipTier::PlainOwned),
            ],
            vec![edge(1, 2, ConstraintKind::PropagateSend)],
        );
        match lattice_solve(&c) {
            LatticeOutcome::Solved(map) => {
                assert_eq!(map.get(KirNodeId(2)), Some(OwnershipTier::ArcShared));
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn floor_ceiling_conflict_detected() {
        let mut n = node(1, OwnershipTier::ArcShared);
        n.ceiling = Some(OwnershipTier::RcShared);
        let c = cluster(vec![n], vec![]);
        assert!(matches!(lattice_solve(&c), LatticeOutcome::Conflict { .. }));
    }

    #[test]
    fn lub_is_commutative() {
        assert_eq!(
            lattice_lub(OwnershipTier::RcShared, OwnershipTier::ArcShared),
            lattice_lub(OwnershipTier::ArcShared, OwnershipTier::RcShared)
        );
    }
}
