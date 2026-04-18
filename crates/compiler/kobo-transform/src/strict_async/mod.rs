use kobo_ir::{AsyncViolationFact, AsyncViolationKind, Kir, KoboMode};

use crate::cfg::{compute_send_requirements, build_cfg, SendRequirements};

/// Check async ownership constraints and return violation facts.
///
/// Contract (k006x_async_executor.md):
/// - In strict mode: non-Send bindings in async context → K0063
/// - In all modes: non-Send bindings crossing spawn → K0060
/// - Non-Sync mutable shared across tasks → K0061
/// - Async code without executor dependency → K0062
///
/// The driver converts these facts to KDiagnostics with proper severity.
pub fn check_strict_async(
    kir: &Kir,
    mode: KoboMode,
    has_executor: bool,
) -> Vec<AsyncViolationFact> {
    let cfg = build_cfg(kir);
    let send_reqs = compute_send_requirements(kir);
    let mut violations = Vec::new();
    let mut has_any_async_binding = false;

    for binding in &kir.transform_facts().bindings {
        if !binding.is_async {
            continue;
        }
        has_any_async_binding = true;

        let needs_send = send_reqs.needs_send(binding.node);
        let shared = &binding.shared_facts;

        // K0060: non-Send binding in async context that needs Send
        // Conservative: any async binding that needs_send but has sharing
        // (Rc-wrapped bindings are not Send)
        if needs_send && shared.needs_sharing && !binding.is_copy_known {
            violations.push(AsyncViolationFact {
                span: binding.span,
                kind: AsyncViolationKind::NonSendCapture {
                    binding_name: binding.binding_name.clone(),
                    binding_id: binding.node,
                },
            });
        }

        // K0061: non-Sync mutable shared binding in async context
        // RefCell is not Sync — if the binding needs mutable sharing in async,
        // the standard Rc<RefCell<T>> wrapper is not safe for cross-task access.
        if needs_send && shared.needs_sharing && shared.needs_mutable_wrapper {
            violations.push(AsyncViolationFact {
                span: binding.span,
                kind: AsyncViolationKind::NonSyncShared {
                    binding_name: binding.binding_name.clone(),
                    binding_id: binding.node,
                },
            });
        }

        // K0063: strict mode — any non-trivial wrapping in async context
        if mode == KoboMode::Strict && needs_send && shared.needs_sharing {
            violations.push(AsyncViolationFact {
                span: binding.span,
                kind: AsyncViolationKind::StrictAsyncViolation {
                    binding_name: binding.binding_name.clone(),
                    binding_id: binding.node,
                    async_fn_span: binding.span,
                },
            });
        }
    }

    // K0062: async code detected but no executor dependency configured
    if has_any_async_binding && !has_executor {
        // Use the span of the first async binding for the diagnostic location
        if let Some(first_async) = kir
            .transform_facts()
            .bindings
            .iter()
            .find(|b| b.is_async)
        {
            violations.push(AsyncViolationFact {
                span: first_async.span,
                kind: AsyncViolationKind::MissingExecutor,
            });
        }
    }

    // Suppress the unused variable warning for cfg — it's used to compute send_reqs
    let _ = &cfg;

    violations
}

/// Convenience: compute SendRequirements for external callers (e.g., tier selection).
pub fn compute_async_send_requirements(kir: &Kir) -> SendRequirements {
    compute_send_requirements(kir)
}

#[cfg(test)]
mod tests;
