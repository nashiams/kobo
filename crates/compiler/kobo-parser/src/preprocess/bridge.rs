/// Bridge module for cross-crate preprocessing coordination.
///
/// When the parser's preprocessor rewrites Kobo keywords into Rust-compatible
/// syntax, some rewrites produce artifacts that downstream crates need to
/// interpret. This module provides bridging types that carry preprocessor
/// decisions forward into transform and codegen without creating direct
/// cross-crate dependencies on internal preprocessor state.

/// A preprocessor decision that affects downstream passes.
#[derive(Clone, Debug)]
pub struct PreprocessBridge {
    /// Original Kobo keyword that triggered the rewrite.
    pub keyword: BridgedKeyword,
    /// Byte offset in the original `.kobo` source.
    pub original_offset: usize,
    /// The marker attribute name generated (e.g., `__kobo_strict`).
    pub marker_attr: String,
    /// Whether this rewrite was inside an async context.
    pub in_async_context: bool,
}

/// Kobo keywords that produce bridged preprocessor decisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgedKeyword {
    /// `@strict` → `#[__kobo_strict]`
    Strict,
    /// `spawn { ... }` → `__kobo_spawn_block!({ ... })`
    Spawn,
    /// `chan<T>` → `__kobo_chan_type!(T)`
    Chan,
    /// `select { ... }` → `__kobo_select_block!({ ... })`
    Select,
}

/// Collect all bridge decisions from a preprocessing pass.
///
/// Called after `preprocess_kobo_keywords` and `preprocess_spawn_blocks`
/// to produce a summary of all keyword rewrites for downstream passes.
pub fn collect_bridge_decisions(
    source: &str,
    strict_offsets: &[(usize, usize)],
    spawn_offsets: &[(usize, usize)],
) -> Vec<PreprocessBridge> {
    let mut decisions = Vec::new();

    for &(start, _end) in strict_offsets {
        decisions.push(PreprocessBridge {
            keyword: BridgedKeyword::Strict,
            original_offset: start,
            marker_attr: "__kobo_strict".to_owned(),
            in_async_context: false, // Refined by transform pass
        });
    }

    for &(start, _end) in spawn_offsets {
        decisions.push(PreprocessBridge {
            keyword: BridgedKeyword::Spawn,
            original_offset: start,
            marker_attr: "__kobo_spawn_block".to_owned(),
            in_async_context: true, // spawn is always async
        });
    }

    decisions.sort_by_key(|d| d.original_offset);
    decisions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_keywords_are_ordered() {
        let decisions = collect_bridge_decisions("", &[(10, 20), (5, 15)], &[(30, 40)]);
        assert_eq!(decisions.len(), 3);
        assert!(decisions[0].original_offset <= decisions[1].original_offset);
        assert!(decisions[1].original_offset <= decisions[2].original_offset);
    }

    #[test]
    fn spawn_is_async_context() {
        let decisions = collect_bridge_decisions("", &[], &[(0, 10)]);
        assert!(decisions[0].in_async_context);
    }
}
