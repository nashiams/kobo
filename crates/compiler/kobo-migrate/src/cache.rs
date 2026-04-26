//! Phase 09: Red-green solver cache.
//!
//! Caches solver results keyed by graph fingerprint so that repeated
//! queries with the same constraint graph skip re-computation.

use std::collections::BTreeMap;

use kobo_ir::SolutionMap;

/// A red-green cache for solver results.
///
/// "Red" entries are stale (input changed), "green" entries are valid.
/// On mutation, all entries are invalidated (turned red).
#[derive(Clone, Debug, Default)]
pub struct SolverCache {
    solution: Option<SolutionMap>,
    fingerprint_cache: BTreeMap<u64, SolutionMap>,
    generation: u64,
}

impl SolverCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the cached full solution, if available.
    pub fn get_solution(&self) -> Option<&SolutionMap> {
        self.solution.as_ref()
    }

    /// Cache the full solution.
    pub fn set_solution(&mut self, map: SolutionMap) {
        self.solution = Some(map);
        self.generation += 1;
    }

    /// Get a cached cluster solution by graph fingerprint.
    pub fn get_by_fingerprint(&self, fingerprint: u64) -> Option<&SolutionMap> {
        self.fingerprint_cache.get(&fingerprint)
    }

    /// Cache a cluster solution by fingerprint.
    pub fn set_by_fingerprint(&mut self, fingerprint: u64, map: SolutionMap) {
        self.fingerprint_cache.insert(fingerprint, map);
    }

    /// Invalidate all cached results.
    pub fn invalidate(&mut self) {
        self.solution = None;
        self.fingerprint_cache.clear();
        self.generation += 1;
    }

    /// Current cache generation (increments on mutation).
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_returns_none() {
        let cache = SolverCache::new();
        assert!(cache.get_solution().is_none());
        assert!(cache.get_by_fingerprint(42).is_none());
    }

    #[test]
    fn set_get_round_trip() {
        let mut cache = SolverCache::new();
        let map = SolutionMap::new();
        cache.set_solution(map.clone());
        assert!(cache.get_solution().is_some());
    }

    #[test]
    fn invalidate_clears_all() {
        let mut cache = SolverCache::new();
        cache.set_solution(SolutionMap::new());
        cache.set_by_fingerprint(99, SolutionMap::new());
        cache.invalidate();
        assert!(cache.get_solution().is_none());
        assert!(cache.get_by_fingerprint(99).is_none());
    }

    #[test]
    fn generation_increments() {
        let mut cache = SolverCache::new();
        let g0 = cache.generation();
        cache.set_solution(SolutionMap::new());
        assert!(cache.generation() > g0);
    }
}
