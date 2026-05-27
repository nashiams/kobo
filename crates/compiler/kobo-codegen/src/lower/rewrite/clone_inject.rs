/// Insert `.clone()` calls for shared bindings crossing spawn boundaries.
///
/// Rules (R-02 resolution):
/// - Binding is Copy type → skip clone, just capture
/// - Binding is shared (tier Arc*/Rc*) → clone before spawn
/// - Binding is consumed (moved into spawn, dead after) → move, no clone
/// - Binding is read-only after spawn → clone (conservative, safe default)
///
/// Generated code pattern:
///   let __data_clone = data.clone();  // inserted before spawn
///   tokio::spawn(async move {
///       // uses __data_clone instead of data
///   });
///
/// Naming: `__{original_name}_clone` to avoid collision.
/// If original is still used after spawn → clone is required.
/// If original is NOT used after spawn → move into spawn, no clone.
use kobo_ir::OwnershipTier;

/// Describes a binding captured by a spawn block for clone injection.
#[derive(Clone, Debug)]
pub(crate) struct CapturedBinding {
    /// Name of the binding in source code.
    pub name: String,
    /// Tier assigned to this binding.
    pub tier: OwnershipTier,
    /// Whether the binding is a known Copy type.
    pub is_copy: bool,
    /// Whether the binding is used after the spawn block.
    pub used_after_spawn: bool,
}

/// Decision for how a captured binding should be handled at a spawn boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureAction {
    /// Move the binding into the spawn — no clone needed.
    Move,
    /// Clone the binding before the spawn — original remains usable.
    Clone,
    /// Copy semantics — just capture, no explicit clone needed.
    Copy,
}

/// Determine how a captured binding should be handled at a spawn boundary.
pub(crate) fn capture_action(binding: &CapturedBinding) -> CaptureAction {
    debug_assert_ne!(binding.tier, OwnershipTier::Undecided);

    // Copy types need no explicit clone — they are cheaply copied.
    if binding.is_copy {
        return CaptureAction::Copy;
    }

    // If the binding is NOT used after spawn, move it in directly.
    if !binding.used_after_spawn {
        return CaptureAction::Move;
    }

    // Used after spawn → must clone to keep the original alive.
    CaptureAction::Clone
}

/// Generate the clone variable name for a binding.
///
/// Pattern: `__{name}_clone` — prefixed to avoid collisions with user code.
pub(crate) fn clone_var_name(binding_name: &str) -> String {
    format!("__{binding_name}_clone")
}

/// Check if a tier is a shared Arc-based tier that can be cheaply cloned.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_arc_tier(tier: OwnershipTier) -> bool {
    matches!(tier, OwnershipTier::ArcShared | OwnershipTier::ArcMutShared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(
        name: &str,
        tier: OwnershipTier,
        is_copy: bool,
        used_after: bool,
    ) -> CapturedBinding {
        CapturedBinding {
            name: name.to_owned(),
            tier,
            is_copy,
            used_after_spawn: used_after,
        }
    }

    #[test]
    fn copy_type_returns_copy_action() {
        let b = binding("count", OwnershipTier::PlainOwned, true, true);
        assert_eq!(capture_action(&b), CaptureAction::Copy);
    }

    #[test]
    fn consumed_binding_returns_move() {
        let b = binding("data", OwnershipTier::PlainOwned, false, false);
        assert_eq!(capture_action(&b), CaptureAction::Move);
    }

    #[test]
    fn shared_binding_used_after_returns_clone() {
        let b = binding("data", OwnershipTier::ArcShared, false, true);
        assert_eq!(capture_action(&b), CaptureAction::Clone);
    }

    #[test]
    fn plain_owned_used_after_returns_clone() {
        let b = binding("data", OwnershipTier::PlainOwned, false, true);
        assert_eq!(capture_action(&b), CaptureAction::Clone);
    }

    #[test]
    fn copy_type_even_if_arc_returns_copy() {
        // Edge case: a Copy + Arc combo (hypothetical) — Copy wins.
        let b = binding("x", OwnershipTier::ArcShared, true, true);
        assert_eq!(capture_action(&b), CaptureAction::Copy);
    }

    #[test]
    fn clone_var_name_format() {
        assert_eq!(clone_var_name("data"), "__data_clone");
        assert_eq!(clone_var_name("config"), "__config_clone");
    }

    #[test]
    fn arc_tier_detection() {
        assert!(is_arc_tier(OwnershipTier::ArcShared));
        assert!(is_arc_tier(OwnershipTier::ArcMutShared));
        assert!(!is_arc_tier(OwnershipTier::RcShared));
        assert!(!is_arc_tier(OwnershipTier::PlainOwned));
    }
}
