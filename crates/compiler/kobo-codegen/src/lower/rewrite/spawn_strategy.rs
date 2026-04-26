/// Select spawn strategy for async tasks: `tokio::spawn` vs `tokio::task::spawn_local`.
///
/// When the solver assigns a non-Send tier (RcShared, RcMutShared) to bindings
/// captured by a spawn block, the task cannot use `tokio::spawn` (requires Send).
/// Instead, we emit `tokio::task::spawn_local` and wrap the surrounding context
/// in a `LocalSet` so the runtime can schedule the non-Send future.
///
/// K0067 is emitted when `spawn_local` is selected but no `LocalSet` context is
/// detected in the enclosing code.

use kobo_ir::{KirNodeId, OwnershipTier, SolutionMap};

/// Strategy chosen for a particular spawn block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpawnStrategy {
    /// All captured bindings are Send — use `tokio::spawn`.
    TokioSpawn,
    /// At least one captured binding is non-Send — use `tokio::task::spawn_local`.
    SpawnLocal,
}

/// Information about a spawn block's capture set.
#[derive(Clone, Debug)]
pub(crate) struct SpawnCaptures {
    pub captured_bindings: Vec<KirNodeId>,
    pub strategy: SpawnStrategy,
    pub non_send_bindings: Vec<KirNodeId>,
}

/// Determine the spawn strategy for a set of captured bindings.
///
/// If any binding is resolved to a non-Send tier (RcShared or RcMutShared),
/// the spawn block must use `spawn_local` instead of `tokio::spawn`.
pub(crate) fn select_spawn_strategy(
    captures: &[KirNodeId],
    solution: &SolutionMap,
) -> SpawnCaptures {
    let mut non_send = Vec::new();
    for &node in captures {
        if let Some(tier) = solution.get(node) {
            if !is_send_tier(tier) {
                non_send.push(node);
            }
        }
    }

    let strategy = if non_send.is_empty() {
        SpawnStrategy::TokioSpawn
    } else {
        SpawnStrategy::SpawnLocal
    };

    SpawnCaptures {
        captured_bindings: captures.to_vec(),
        strategy,
        non_send_bindings: non_send,
    }
}

/// Returns true if the tier produces a Send wrapper type.
fn is_send_tier(tier: OwnershipTier) -> bool {
    matches!(
        tier,
        OwnershipTier::PlainOwned
            | OwnershipTier::BoxOwned
            | OwnershipTier::ArcShared
            | OwnershipTier::ArcMutShared
    )
}

/// Generate the spawn expression text for a given strategy.
pub(crate) fn spawn_call_prefix(strategy: SpawnStrategy) -> &'static str {
    match strategy {
        SpawnStrategy::TokioSpawn => "tokio::spawn(async move",
        SpawnStrategy::SpawnLocal => "tokio::task::spawn_local(async move",
    }
}

/// Check if a LocalSet wrapper is needed at the function level.
///
/// Returns true if any spawn block in the function uses `SpawnLocal` strategy.
pub(crate) fn needs_local_set(spawn_blocks: &[SpawnCaptures]) -> bool {
    spawn_blocks
        .iter()
        .any(|s| s.strategy == SpawnStrategy::SpawnLocal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_tiers_use_tokio_spawn() {
        let solution = SolutionMap::new();
        let captures = select_spawn_strategy(&[], &solution);
        assert_eq!(captures.strategy, SpawnStrategy::TokioSpawn);
    }

    #[test]
    fn spawn_call_prefix_matches_strategy() {
        assert!(spawn_call_prefix(SpawnStrategy::TokioSpawn).contains("tokio::spawn"));
        assert!(spawn_call_prefix(SpawnStrategy::SpawnLocal).contains("spawn_local"));
    }
}
