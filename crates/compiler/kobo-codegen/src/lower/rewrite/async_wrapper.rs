//! Select the correct Rust wrapper type for async bindings.
//!
//! Mapping table (v0.8):
//!   OwnershipTier::ArcShared (read-only) → Arc<T>
//!   OwnershipTier::ArcMutShared (mutable) → Arc<tokio::sync::RwLock<T>>
//!   OwnershipTier::ArcMutShared (counter-like) → Arc<AtomicU64>  // reserved for future specialized lowering
//!
//! HARD RULES:
//!   - NEVER generate Arc<std::sync::Mutex<T>>
//!   - NEVER generate std::sync::Mutex in async context
//!   - ALWAYS tokio::sync::RwLock when .await may be held
//!
//! In codegen, ArcMutShared in async context emits:
//!   Arc::new(tokio::sync::RwLock::new(value))
//! And access patterns emit:
//!   .read().await  (for reads)
//!   .write().await (for writes)

#![cfg_attr(not(test), allow(dead_code))]
use kobo_ir::OwnershipTier;

/// Wrapper kind for async bindings — determines what Rust type is emitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AsyncWrapperKind {
    /// `Arc<T>` — read-only shared across tasks.
    ArcReadOnly,
    /// `Arc<tokio::sync::RwLock<T>>` — mutable shared across tasks.
    ArcRwLock,
    /// No async wrapper needed — binding stays as-is.
    None,
}

/// Select the correct async wrapper kind for a given ownership tier.
///
/// Only `ArcShared` and `ArcMutShared` require async wrappers.
/// Other tiers (including Rc-based) are not valid in Send contexts
/// and do not receive async wrappers.
pub(crate) fn async_wrapper_kind(tier: OwnershipTier, is_async: bool) -> AsyncWrapperKind {
    if !is_async {
        return AsyncWrapperKind::None;
    }
    match tier {
        OwnershipTier::ArcShared => AsyncWrapperKind::ArcReadOnly,
        OwnershipTier::ArcMutShared => AsyncWrapperKind::ArcRwLock,
        _ => AsyncWrapperKind::None,
    }
}

/// Return the access expression suffix for reading an async-wrapped binding.
///
/// - `ArcReadOnly` → no suffix (deref via Arc)
/// - `ArcRwLock` → `.read().await`
pub(crate) fn async_read_access(kind: AsyncWrapperKind) -> &'static str {
    match kind {
        AsyncWrapperKind::ArcRwLock => ".read().await",
        _ => "",
    }
}

/// Return the access expression suffix for writing to an async-wrapped binding.
///
/// - `ArcReadOnly` → panic (cannot write to read-only Arc)
/// - `ArcRwLock` → `.write().await`
pub(crate) fn async_write_access(kind: AsyncWrapperKind) -> &'static str {
    match kind {
        AsyncWrapperKind::ArcRwLock => ".write().await",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_shared_in_async_is_read_only() {
        let kind = async_wrapper_kind(OwnershipTier::ArcShared, true);
        assert_eq!(kind, AsyncWrapperKind::ArcReadOnly);
    }

    #[test]
    fn arc_mut_shared_in_async_is_rwlock() {
        let kind = async_wrapper_kind(OwnershipTier::ArcMutShared, true);
        assert_eq!(kind, AsyncWrapperKind::ArcRwLock);
    }

    #[test]
    fn non_async_returns_none() {
        let kind = async_wrapper_kind(OwnershipTier::ArcMutShared, false);
        assert_eq!(kind, AsyncWrapperKind::None);
    }

    #[test]
    fn plain_tier_returns_none() {
        let kind = async_wrapper_kind(OwnershipTier::PlainOwned, true);
        assert_eq!(kind, AsyncWrapperKind::None);
    }

    #[test]
    fn rwlock_read_access() {
        let access = async_read_access(AsyncWrapperKind::ArcRwLock);
        assert_eq!(access, ".read().await");
    }

    #[test]
    fn rwlock_write_access() {
        let access = async_write_access(AsyncWrapperKind::ArcRwLock);
        assert_eq!(access, ".write().await");
    }

    #[test]
    fn read_only_no_access_suffix() {
        let access = async_read_access(AsyncWrapperKind::ArcReadOnly);
        assert_eq!(access, "");
    }

    #[test]
    fn no_wrapper_no_access_suffix() {
        let access = async_read_access(AsyncWrapperKind::None);
        assert_eq!(access, "");
    }
}
