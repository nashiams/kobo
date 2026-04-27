//! Phase 09: MigrateCtxt — shared context for the migration pipeline.
//!
//! Holds the KIR, configuration, summaries, and caches in one place
//! so that every phase can query the same state.

use std::path::{Path, PathBuf};

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
    cache_dir: Option<PathBuf>,
}

impl MigrateCtxt {
    /// Create a new migration context from a KIR and config.
    pub fn new(kir: Kir, config: GreedyConfig) -> Self {
        Self::new_inner(kir, config, None)
    }

    /// Create a migration context with an on-disk solver cache directory.
    pub fn new_with_cache_dir(
        kir: Kir,
        config: GreedyConfig,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self::new_inner(kir, config, Some(cache_dir.into()))
    }

    fn new_inner(kir: Kir, config: GreedyConfig, cache_dir: Option<PathBuf>) -> Self {
        let summaries = crate::summaries::build_summaries(&kir);
        let call_graph = crate::call_graph::build_call_graph(&kir);

        Self {
            kir,
            config,
            summaries,
            call_graph,
            cache: SolverCache::new(),
            cache_dir,
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

    /// Optional on-disk cache directory for this context.
    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctxt_creation_with_empty_kir() {
        let kir = Kir::default();
        let config = GreedyConfig::default();
        let ctxt = MigrateCtxt::new(kir, config);
        assert!(ctxt.summaries().is_empty());
        assert_eq!(ctxt.call_graph().node_count(), 0);
    }
}
