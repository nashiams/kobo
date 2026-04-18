use kobo_ir::{AsyncViolationFact, AsyncViolationKind, Kir, KoboMode};

use crate::cfg::{compute_send_requirements, build_cfg, SendRequirements};

/// Check async ownership constraints and return violation facts.
///
/// Contract (k006x_async_executor.md):
/// - In strict mode: non-Send bindings in async context → K0063
/// - In all modes: non-Send bindings crossing spawn → K0060
/// - Non-Sync shared across tasks → K0061
///
/// The driver converts these facts to KDiagnostics with proper severity.
pub fn check_strict_async(kir: &Kir, mode: KoboMode) -> Vec<AsyncViolationFact> {
    let cfg = build_cfg(kir);
    let send_reqs = compute_send_requirements(kir);
    let mut violations = Vec::new();

    for binding in &kir.transform_facts().bindings {
        if !binding.is_async {
            continue;
        }

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
