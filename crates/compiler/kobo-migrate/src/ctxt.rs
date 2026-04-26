//! Phase 09: MigrateCtxt — shared context for the migration pipeline.
//!
//! Holds the KIR, configuration, summaries, and caches in one place
//! so that every phase can query the same state.

use kobo_ir::Kir;

use crate::cache::SolverCache;
use crate::call_graph::CallGraph;
use crate::greedy::GreedyConfig;
use crate::summaries::SummaryTable;

/// Shared context for the constraint-solving pipeline.
pub struct MigrateCtxt {
    pub kir: Kir,
    pub config: GreedyConfig,
    pub summaries: SummaryTable,
    pub call_graph: CallGraph,
    pub cache: SolverCache,
}

impl MigrateCtxt {
    /// Create a new migration context from a KIR and config.
    pub fn new(kir: Kir, config: GreedyConfig) -> Self {
        let summaries = crate::summaries::build_summaries(&kir);
        let call_graph = crate::call_graph::build_call_graph(&kir);

        Self {
            kir,
            config,
            summaries,
            call_graph,
            cache: SolverCache::new(),
        }
    }

    /// Access the KIR.
    pub fn kir(&self) -> &Kir {
        &self.kir
    }

    /// Access the greedy config.
    pub fn config(&self) -> &GreedyConfig {
        &self.config
    }

    /// Access the summary table.
    pub fn summaries(&self) -> &SummaryTable {
        &self.summaries
    }

    /// Access the call graph.
    pub fn call_graph(&self) -> &CallGraph {
        &self.call_graph
    }

    /// Access the solver cache.
    pub fn cache(&self) -> &SolverCache {
        &self.cache
    }

    /// Mutable access to the cache.
    pub fn cache_mut(&mut self) -> &mut SolverCache {
        &mut self.cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctxt_creation_with_empty_kir() {
        let kir = Kir::default();
        let config = GreedyConfig {
            solver_cluster_limit: 256,
            solver_budget_seconds: 5.0,
        };
        let ctxt = MigrateCtxt::new(kir, config);
        assert!(ctxt.summaries().is_empty());
        assert_eq!(ctxt.call_graph().node_count(), 0);
    }
}
