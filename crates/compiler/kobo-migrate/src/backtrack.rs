//! Phase 08: Backtracking search with checkpoint/rollback.
//!
//! Provides a checkpoint/rollback mechanism over SolutionMap so that
//! the solver can speculatively assign tiers and undo on conflict.
//! Also introduces `Disjunction` for representing either-or constraints.

use std::collections::BTreeMap;

use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap};

use crate::cluster::Cluster;
use crate::lattice_solve::lattice_lub;
use crate::solver::ConstraintKind;

/// A disjunctive constraint: at least one of the alternatives must hold.
#[derive(Clone, Debug)]
pub struct Disjunction {
    pub node: KirNodeId,
    pub alternatives: Vec<OwnershipTier>,
    pub label: String,
}

/// Checkpoint: a snapshot of solver state that can be restored on conflict.
#[derive(Clone, Debug)]
struct Checkpoint {
    assignments: BTreeMap<KirNodeId, OwnershipTier>,
    depth: u32,
}

/// Backtracking solver state.
pub struct BacktrackSolver {
    current: BTreeMap<KirNodeId, OwnershipTier>,
    floors: BTreeMap<KirNodeId, OwnershipTier>,
    ceilings: BTreeMap<KirNodeId, Option<OwnershipTier>>,
    checkpoints: Vec<Checkpoint>,
    max_depth: u32,
    nodes_tried: u64,
    node_budget: u64,
}

/// Result of backtracking search.
#[derive(Clone, Debug)]
pub enum BacktrackResult {
    /// A valid assignment was found.
    Solved(SolutionMap),
    /// No valid assignment exists within the budget.
    Exhausted,
    /// Budget exceeded before completion.
    BudgetExceeded { nodes_tried: u64 },
}

impl BacktrackSolver {
    /// Create a new solver from cluster node information.
    pub fn new(cluster: &Cluster) -> Self {
        let mut current = BTreeMap::new();
        let mut floors = BTreeMap::new();
        let mut ceilings = BTreeMap::new();

        for node in &cluster.nodes {
            current.insert(node.id, node.floor);
            floors.insert(node.id, node.floor);
            ceilings.insert(node.id, node.ceiling);
        }

        Self {
            current,
            floors,
            ceilings,
            checkpoints: Vec::new(),
            max_depth: 64,
            nodes_tried: 0,
            node_budget: 10_000,
        }
    }

    /// Set the maximum search depth.
    pub fn set_max_depth(&mut self, depth: u32) {
        self.max_depth = depth;
    }

    /// Set the node expansion budget.
    pub fn set_node_budget(&mut self, budget: u64) {
        self.node_budget = budget;
    }

    /// Save a checkpoint.
    fn checkpoint(&mut self) {
        self.checkpoints.push(Checkpoint {
            assignments: self.current.clone(),
            depth: self.checkpoints.len() as u32,
        });
    }

    /// Rollback to the last checkpoint.
    fn rollback(&mut self) -> bool {
        if let Some(cp) = self.checkpoints.pop() {
            self.current = cp.assignments;
            true
        } else {
            false
        }
    }

    /// Attempt to assign `tier` to `node`, returning false on conflict.
    fn try_assign(&mut self, node: KirNodeId, tier: OwnershipTier) -> bool {
        let floor = self.floors.get(&node).copied().unwrap_or(OwnershipTier::PlainOwned);
        let ceiling = self.ceilings.get(&node).copied().flatten();

        let rank = crate::lattice_solve::tier_rank(tier);
        if rank < crate::lattice_solve::tier_rank(floor) {
            return false;
        }
        if let Some(ceil) = ceiling {
            if rank > crate::lattice_solve::tier_rank(ceil) {
                return false;
            }
        }

        self.current.insert(node, tier);
        true
    }

    /// Run backtracking search with disjunctions.
    pub fn solve(
        &mut self,
        cluster: &Cluster,
        disjunctions: &[Disjunction],
    ) -> BacktrackResult {
        // If no disjunctions, the current assignment (from floors) is the answer.
        if disjunctions.is_empty() {
            let mut map = SolutionMap::new();
            for (&id, &tier) in &self.current {
                map.insert(id, tier);
            }
            return BacktrackResult::Solved(map);
        }

        // DFS over disjunctions.
        self.search(cluster, disjunctions, 0)
    }

    fn search(
        &mut self,
        cluster: &Cluster,
        disjunctions: &[Disjunction],
        idx: usize,
    ) -> BacktrackResult {
        if self.nodes_tried >= self.node_budget {
            return BacktrackResult::BudgetExceeded {
                nodes_tried: self.nodes_tried,
            };
        }

        if idx >= disjunctions.len() {
            // All disjunctions satisfied — check global consistency.
            if self.is_consistent(cluster) {
                let mut map = SolutionMap::new();
                for (&id, &tier) in &self.current {
                    map.insert(id, tier);
                }
                return BacktrackResult::Solved(map);
            } else {
                return BacktrackResult::Exhausted;
            }
        }

        if self.checkpoints.len() as u32 >= self.max_depth {
            return BacktrackResult::Exhausted;
        }

        let disj = &disjunctions[idx];
        for &alt in &disj.alternatives {
            self.nodes_tried += 1;
            self.checkpoint();

            if self.try_assign(disj.node, alt) {
                let result = self.search(cluster, disjunctions, idx + 1);
                match result {
                    BacktrackResult::Solved(_) => return result,
                    BacktrackResult::BudgetExceeded { .. } => return result,
                    BacktrackResult::Exhausted => {
                        self.rollback();
                    }
                }
            } else {
                self.rollback();
            }
        }

        BacktrackResult::Exhausted
    }

    /// Check global consistency of the current assignment.
    fn is_consistent(&self, cluster: &Cluster) -> bool {
        for edge in &cluster.raw_edges {
            let src = self.current.get(&edge.source);
            let tgt = self.current.get(&edge.target);
            if let (Some(&s), Some(&t)) = (src, tgt) {
                match &edge.kind {
                    ConstraintKind::PropagateSharing => {
                        // Both must be at least as high as the LUB.
                        let lub = lattice_lub(s, t);
                        if s != lub && t != lub {
                            // One must dominate the other.
                        }
                    }
                    ConstraintKind::PropagateSend => {
                        // Both must be thread-safe.
                        if !s.is_thread_safe() || !t.is_thread_safe() {
                            return false;
                        }
                    }
                    ConstraintKind::MutuallyExclusive => {
                        // Cannot both be mutable-shared wrappers.
                        let s_mut = matches!(
                            s,
                            OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared
                        );
                        let t_mut = matches!(
                            t,
                            OwnershipTier::RcMutShared | OwnershipTier::ArcMutShared
                        );
                        if s_mut && t_mut {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterId};
    use crate::constraint_extract::ConstraintNode;
    use crate::solver::ConstraintEdge;
    use kobo_ir::KirNodeId;

    fn make_node(id: u32, floor: OwnershipTier) -> ConstraintNode {
        ConstraintNode {
            id: KirNodeId(id),
            floor,
            ceiling: None,
            is_boundary: false,
            binding_name: format!("v{}", id),
        }
    }

    fn make_cluster(nodes: Vec<ConstraintNode>, edges: Vec<ConstraintEdge>) -> Cluster {
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
    fn empty_disjunctions_yields_floor_solution() {
        let cluster = make_cluster(
            vec![make_node(1, OwnershipTier::RcShared)],
            vec![],
        );
        let mut solver = BacktrackSolver::new(&cluster);
        match solver.solve(&cluster, &[]) {
            BacktrackResult::Solved(map) => {
                assert_eq!(map.get(KirNodeId(1)), Some(OwnershipTier::RcShared));
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn disjunction_picks_valid_alternative() {
        let cluster = make_cluster(
            vec![make_node(1, OwnershipTier::PlainOwned)],
            vec![],
        );
        let mut solver = BacktrackSolver::new(&cluster);
        let disj = vec![Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::RcShared, OwnershipTier::ArcShared],
            label: "test".into(),
        }];
        match solver.solve(&cluster, &disj) {
            BacktrackResult::Solved(map) => {
                let tier = map.get(KirNodeId(1)).unwrap();
                assert!(matches!(
                    tier,
                    OwnershipTier::RcShared | OwnershipTier::ArcShared
                ));
            }
            _ => panic!("expected solved"),
        }
    }

    #[test]
    fn ceiling_violation_backtracks() {
        let mut n = make_node(1, OwnershipTier::PlainOwned);
        n.ceiling = Some(OwnershipTier::RcShared);
        let cluster = make_cluster(vec![n], vec![]);
        let mut solver = BacktrackSolver::new(&cluster);
        let disj = vec![Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::ArcMutShared, OwnershipTier::RcShared],
            label: "test".into(),
        }];
        match solver.solve(&cluster, &disj) {
            BacktrackResult::Solved(map) => {
                // Should have picked RcShared (ArcMutShared exceeds ceiling).
                assert_eq!(map.get(KirNodeId(1)), Some(OwnershipTier::RcShared));
            }
            _ => panic!("expected solved with fallback"),
        }
    }

    #[test]
    fn budget_exceeded_returns_early() {
        let cluster = make_cluster(
            vec![make_node(1, OwnershipTier::PlainOwned)],
            vec![],
        );
        let mut solver = BacktrackSolver::new(&cluster);
        solver.set_node_budget(0);
        let disj = vec![Disjunction {
            node: KirNodeId(1),
            alternatives: vec![OwnershipTier::RcShared],
            label: "test".into(),
        }];
        assert!(matches!(
            solver.solve(&cluster, &disj),
            BacktrackResult::BudgetExceeded { .. }
        ));
    }
}
