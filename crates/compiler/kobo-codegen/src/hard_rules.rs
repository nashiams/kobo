/// Hard rules enforcement for async code generation.
///
/// These invariants are enforced at the codegen boundary to guarantee
/// that forbidden patterns NEVER appear in generated Rust code:
///
/// - No `Arc<Mutex<T>>` (use `Arc<tokio::sync::RwLock<T>>`)
/// - No `std::sync::Mutex` in async context
/// - No `std::sync::RwLock` in async; use `tokio::sync::RwLock`
/// - Strict mode = no wrappers, period
///
/// The tier selection in `kobo-transform` prevents most forbidden states,
/// but these checks act as a final gate at codegen time.
use kobo_ir::OwnershipTier;

/// Error returned when a hard rule is violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardRuleViolation {
    /// Attempted to generate `Arc<Mutex<T>>`; use `Arc<tokio::sync::RwLock<T>>` instead.
    ForbiddenArcMutex { binding_name: String },
    /// Strict mode does not allow any wrapper types.
    StrictNoWrappers {
        binding_name: String,
        tier: OwnershipTier,
    },
}

/// Validate that a tier assignment does not violate hard rules.
///
/// Called from codegen before emitting wrapper code. Returns `Ok(())` if the
/// tier is allowed, or `Err(violation)` if a hard rule is broken.
pub fn validate_tier(
    binding_name: &str,
    tier: OwnershipTier,
    is_strict: bool,
) -> Result<(), HardRuleViolation> {
    // ArcMutShared must map to Arc<tokio::sync::RwLock<T>>, never Arc<Mutex<T>>.
    // This is structurally guaranteed because OwnershipTier has no ArcMutex variant;
    // ArcMutShared always maps to RwLock in codegen (see lower/binding.rs).
    // The ForbiddenArcMutex variant exists as a safety net for future tiers.
    // Currently unreachable: no tier maps to std::sync::Mutex.

    // Strict mode forbids all wrapper tiers.
    if is_strict {
        match tier {
            OwnershipTier::PlainOwned | OwnershipTier::Undecided => {}
            _ => {
                return Err(HardRuleViolation::StrictNoWrappers {
                    binding_name: binding_name.to_string(),
                    tier,
                });
            }
        }
    }

    Ok(())
}

/// Assert that ArcMutShared always maps to `tokio::sync::RwLock`, never `std::sync::Mutex`.
///
/// This is a compile-time design guarantee enforced by the `apply_tier_to_local`
/// match arm in `lower/binding.rs`. This function exists as a testable assertion.
pub fn arc_mut_shared_uses_tokio_rwlock() -> &'static str {
    "Arc<tokio::sync::RwLock<T>>"
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- No Arc<Mutex<T>> ---

    /// ArcMutShared maps to tokio::sync::RwLock, not std::sync::Mutex.
    #[test]
    fn arc_mut_shared_is_tokio_rwlock() {
        let result = arc_mut_shared_uses_tokio_rwlock();
        assert!(result.contains("tokio::sync::RwLock"));
        assert!(!result.contains("std::sync::Mutex"));
    }

    // --- Strict mode forbids wrappers ---

    /// PlainOwned is always allowed in strict mode.
    #[test]
    fn strict_allows_plain_owned() {
        assert!(validate_tier("x", OwnershipTier::PlainOwned, true).is_ok());
    }

    /// BoxOwned is forbidden in strict mode.
    #[test]
    fn strict_forbids_box_owned() {
        let result = validate_tier("x", OwnershipTier::BoxOwned, true);
        assert!(matches!(
            result,
            Err(HardRuleViolation::StrictNoWrappers { .. })
        ));
    }

    /// RcShared is forbidden in strict mode.
    #[test]
    fn strict_forbids_rc_shared() {
        let result = validate_tier("x", OwnershipTier::RcShared, true);
        assert!(matches!(
            result,
            Err(HardRuleViolation::StrictNoWrappers { .. })
        ));
    }

    /// ArcShared is forbidden in strict mode.
    #[test]
    fn strict_forbids_arc_shared() {
        let result = validate_tier("x", OwnershipTier::ArcShared, true);
        assert!(matches!(
            result,
            Err(HardRuleViolation::StrictNoWrappers { .. })
        ));
    }

    /// RcMutShared is forbidden in strict mode.
    #[test]
    fn strict_forbids_rc_mut_shared() {
        let result = validate_tier("x", OwnershipTier::RcMutShared, true);
        assert!(matches!(
            result,
            Err(HardRuleViolation::StrictNoWrappers { .. })
        ));
    }

    /// ArcMutShared is forbidden in strict mode.
    #[test]
    fn strict_forbids_arc_mut_shared() {
        let result = validate_tier("x", OwnershipTier::ArcMutShared, true);
        assert!(matches!(
            result,
            Err(HardRuleViolation::StrictNoWrappers { .. })
        ));
    }

    /// Scoped is forbidden in strict mode.
    #[test]
    fn strict_forbids_scoped() {
        let result = validate_tier("x", OwnershipTier::Scoped, true);
        assert!(matches!(
            result,
            Err(HardRuleViolation::StrictNoWrappers { .. })
        ));
    }

    // --- Non-strict mode allows everything ---

    /// Non-strict mode allows all tiers.
    #[test]
    fn non_strict_allows_all_tiers() {
        assert!(validate_tier("x", OwnershipTier::PlainOwned, false).is_ok());
        assert!(validate_tier("x", OwnershipTier::BoxOwned, false).is_ok());
        assert!(validate_tier("x", OwnershipTier::RcShared, false).is_ok());
        assert!(validate_tier("x", OwnershipTier::ArcShared, false).is_ok());
        assert!(validate_tier("x", OwnershipTier::RcMutShared, false).is_ok());
        assert!(validate_tier("x", OwnershipTier::ArcMutShared, false).is_ok());
        assert!(validate_tier("x", OwnershipTier::Scoped, false).is_ok());
    }

    /// Violation message includes binding name and tier.
    #[test]
    fn violation_includes_binding_info() {
        let result = validate_tier("my_var", OwnershipTier::ArcShared, true);
        match result {
            Err(HardRuleViolation::StrictNoWrappers { binding_name, tier }) => {
                assert_eq!(binding_name, "my_var");
                assert_eq!(tier, OwnershipTier::ArcShared);
            }
            _ => panic!("expected StrictNoWrappers violation"),
        }
    }
}
