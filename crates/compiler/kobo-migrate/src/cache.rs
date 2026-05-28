//! Stage: Red-green solver cache.
//!
//! Caches solver results keyed by graph fingerprint so that repeated
//! queries with the same constraint graph skip re-computation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kobo_ir::SolutionMap;

use crate::solver::SolveOutcome;

const CACHE_VERSION: &str = "solver-cache-v1";

/// A red-green cache for solver results.
///
/// "Red" entries are stale (input changed), "green" entries are valid.
/// On mutation, all entries are invalidated (turned red).
#[derive(Clone, Debug, Default)]
pub struct SolverCache {
    solution: Option<SolutionMap>,
    outcome: Option<SolveOutcome>,
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

    /// Get the cached full solve outcome, if available.
    pub fn get_outcome(&self) -> Option<&SolveOutcome> {
        self.outcome.as_ref()
    }

    /// Cache the full solve outcome.
    pub fn set_outcome(&mut self, outcome: SolveOutcome) {
        self.outcome = Some(outcome);
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
        self.outcome = None;
        self.fingerprint_cache.clear();
        self.generation += 1;
    }

    /// Current cache generation (increments on mutation).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Load a complete Unique outcome from the on-disk cache.
    ///
    /// Only Unique outcomes are reused: they contain a complete verified map.
    /// Non-unique outcomes intentionally stay uncached so diagnostics are
    /// rebuilt against the current KIR and never go stale.
    pub fn load_unique_outcome(cache_dir: &Path, fingerprint: &str) -> Option<SolveOutcome> {
        let path = cache_file_path(cache_dir, fingerprint);
        let content = std::fs::read_to_string(path).ok()?;
        parse_cache_entry(&content, fingerprint)
    }

    /// Persist a complete Unique outcome to disk. Returns true when something
    /// was written and false when the outcome is intentionally not cacheable.
    pub fn save_unique_outcome(
        cache_dir: &Path,
        fingerprint: &str,
        outcome: &SolveOutcome,
    ) -> std::io::Result<bool> {
        let SolveOutcome::Unique(map) = outcome else {
            return Ok(false);
        };

        std::fs::create_dir_all(cache_dir)?;
        let path = cache_file_path(cache_dir, fingerprint);
        let tmp_path = temporary_cache_file_path(cache_dir, fingerprint);
        let mut body = String::new();
        body.push_str(CACHE_VERSION);
        body.push('\n');
        body.push_str("fingerprint ");
        body.push_str(fingerprint);
        body.push('\n');
        body.push_str("outcome Unique\n");

        let mut entries: Vec<_> = map.iter().collect();
        entries.sort_by_key(|(node_id, tier)| (node_id.0, *tier));
        for (node_id, tier) in entries {
            body.push_str("entry ");
            body.push_str(&node_id.0.to_string());
            body.push(' ');
            body.push_str(tier_name(tier));
            body.push('\n');
        }

        std::fs::write(&tmp_path, body)?;
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        std::fs::rename(tmp_path, path)?;
        Ok(true)
    }
}

fn cache_file_path(cache_dir: &Path, fingerprint: &str) -> PathBuf {
    cache_dir.join(format!("{fingerprint}.solution"))
}

fn temporary_cache_file_path(cache_dir: &Path, fingerprint: &str) -> PathBuf {
    cache_dir.join(format!("{fingerprint}.solution.tmp"))
}

fn parse_cache_entry(content: &str, expected_fingerprint: &str) -> Option<SolveOutcome> {
    let mut lines = content.lines();
    if lines.next()? != CACHE_VERSION {
        return None;
    }

    let mut fingerprint_seen = false;
    let mut unique_seen = false;
    let mut map = SolutionMap::new();

    for line in lines {
        let mut parts = line.split_whitespace();
        match parts.next()? {
            "fingerprint" => {
                let fingerprint = parts.next()?;
                if fingerprint != expected_fingerprint {
                    return None;
                }
                fingerprint_seen = true;
            }
            "outcome" => {
                unique_seen = parts.next()? == "Unique";
                if !unique_seen {
                    return None;
                }
            }
            "entry" => {
                let node_id: u32 = parts.next()?.parse().ok()?;
                let tier = parse_tier(parts.next()?)?;
                map.insert(kobo_ir::KirNodeId(node_id), tier);
            }
            _ => return None,
        }
    }

    if fingerprint_seen && unique_seen {
        Some(SolveOutcome::Unique(map))
    } else {
        None
    }
}

fn tier_name(tier: kobo_ir::OwnershipTier) -> &'static str {
    match tier {
        kobo_ir::OwnershipTier::PlainOwned => "PlainOwned",
        kobo_ir::OwnershipTier::BoxOwned => "BoxOwned",
        kobo_ir::OwnershipTier::RcShared => "RcShared",
        kobo_ir::OwnershipTier::ArcShared => "ArcShared",
        kobo_ir::OwnershipTier::RcMutShared => "RcMutShared",
        kobo_ir::OwnershipTier::ArcMutShared => "ArcMutShared",
        kobo_ir::OwnershipTier::Scoped => "Scoped",
        kobo_ir::OwnershipTier::Undecided => "Undecided",
    }
}

fn parse_tier(name: &str) -> Option<kobo_ir::OwnershipTier> {
    match name {
        "PlainOwned" => Some(kobo_ir::OwnershipTier::PlainOwned),
        "BoxOwned" => Some(kobo_ir::OwnershipTier::BoxOwned),
        "RcShared" => Some(kobo_ir::OwnershipTier::RcShared),
        "ArcShared" => Some(kobo_ir::OwnershipTier::ArcShared),
        "RcMutShared" => Some(kobo_ir::OwnershipTier::RcMutShared),
        "ArcMutShared" => Some(kobo_ir::OwnershipTier::ArcMutShared),
        "Scoped" => Some(kobo_ir::OwnershipTier::Scoped),
        "Undecided" => Some(kobo_ir::OwnershipTier::Undecided),
        _ => None,
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
        cache.set_outcome(SolveOutcome::Unique(SolutionMap::new()));
        cache.set_by_fingerprint(99, SolutionMap::new());
        cache.invalidate();
        assert!(cache.get_solution().is_none());
        assert!(cache.get_outcome().is_none());
        assert!(cache.get_by_fingerprint(99).is_none());
    }

    #[test]
    fn generation_increments() {
        let mut cache = SolverCache::new();
        let g0 = cache.generation();
        cache.set_solution(SolutionMap::new());
        assert!(cache.generation() > g0);
    }

    #[test]
    fn disk_cache_round_trips_unique_outcome() {
        let dir =
            std::env::temp_dir().join(format!("kobo-solver-cache-test-{}", std::process::id()));
        let fingerprint = "0123456789abcdef";
        let mut map = SolutionMap::new();
        map.insert(kobo_ir::KirNodeId(10), kobo_ir::OwnershipTier::ArcShared);
        let outcome = SolveOutcome::Unique(map);

        let wrote = SolverCache::save_unique_outcome(&dir, fingerprint, &outcome).unwrap();
        assert!(wrote);
        let loaded = SolverCache::load_unique_outcome(&dir, fingerprint).unwrap();
        match loaded {
            SolveOutcome::Unique(map) => {
                assert_eq!(
                    map.get(kobo_ir::KirNodeId(10)),
                    Some(kobo_ir::OwnershipTier::ArcShared)
                );
            }
            _ => panic!("expected cached unique outcome"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn disk_cache_rejects_mismatched_fingerprint() {
        let dir = std::env::temp_dir().join(format!(
            "kobo-solver-cache-mismatch-test-{}",
            std::process::id()
        ));
        let outcome = SolveOutcome::Unique(SolutionMap::new());
        SolverCache::save_unique_outcome(&dir, "aaaa", &outcome).unwrap();
        assert!(SolverCache::load_unique_outcome(&dir, "bbbb").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
