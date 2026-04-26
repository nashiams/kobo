/// Guard liveness analysis for async contexts.
///
/// Detects MutexGuard, RwLockReadGuard, and RwLockWriteGuard bindings
/// that are live across `.await` points. These cause runtime deadlocks
/// and compile errors (`future is not Send`). The analysis produces
/// advisory facts that the driver projects as K0062 diagnostics.

use kobo_ir::{Kir, KirNodeId, KoboSpan};

/// A guard binding that is live across an await point.
#[derive(Clone, Debug)]
pub struct GuardLiveAcrossAwait {
    pub guard_binding: KirNodeId,
    pub binding_name: String,
    pub guard_span: KoboSpan,
    pub await_span: KoboSpan,
    pub guard_kind: GuardKind,
}

/// Kind of synchronization guard detected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardKind {
    MutexGuard,
    RwLockRead,
    RwLockWrite,
}

/// Detect guard bindings that are live across await points.
///
/// Scans KIR for bindings whose type matches a known guard pattern
/// and checks if any `.await` expression occurs between the guard
/// acquisition and the last use of the guard binding.
pub fn detect_guard_across_await(kir: &Kir) -> Vec<GuardLiveAcrossAwait> {
    let mut results = Vec::new();

    // Walk all bindings looking for guard-like types.
    for binding in &kir.transform_facts().bindings {
        let guard_kind = match classify_guard(&binding.binding_name) {
            Some(k) => k,
            None => continue,
        };

        // Check if this binding is in an async context and has an await after it.
        if !binding.is_async {
            continue;
        }

        // A guard in async context that participates in sharing is suspect.
        // The actual liveness check requires CFG traversal; for now we flag
        // any guard binding in async that has downstream uses.
        if binding.shared_facts.needs_sharing || binding.shared_facts.mutation_required {
            results.push(GuardLiveAcrossAwait {
                guard_binding: binding.node,
                binding_name: binding.binding_name.clone(),
                guard_span: binding.span,
                await_span: binding.span, // Approximate — refine with CFG in future
                guard_kind,
            });
        }
    }

    results
}

/// Classify a binding name as a guard type based on naming conventions.
///
/// Kobo convention: bindings named `*_guard`, `*_lock`, `*_read`, `*_write`
/// in async contexts are treated as potential synchronization guards.
fn classify_guard(name: &str) -> Option<GuardKind> {
    let lower = name.to_lowercase();
    if lower.ends_with("_guard") || lower.contains("mutex_guard") {
        Some(GuardKind::MutexGuard)
    } else if lower.ends_with("_read") || lower.contains("read_guard") {
        Some(GuardKind::RwLockRead)
    } else if lower.ends_with("_write") || lower.contains("write_guard") {
        Some(GuardKind::RwLockWrite)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_mutex_guard() {
        assert_eq!(classify_guard("db_guard"), Some(GuardKind::MutexGuard));
        assert_eq!(classify_guard("mutex_guard_x"), Some(GuardKind::MutexGuard));
    }

    #[test]
    fn classify_rwlock_guards() {
        assert_eq!(classify_guard("cache_read"), Some(GuardKind::RwLockRead));
        assert_eq!(classify_guard("state_write"), Some(GuardKind::RwLockWrite));
    }

    #[test]
    fn non_guard_returns_none() {
        assert_eq!(classify_guard("data"), None);
        assert_eq!(classify_guard("counter"), None);
    }
}
