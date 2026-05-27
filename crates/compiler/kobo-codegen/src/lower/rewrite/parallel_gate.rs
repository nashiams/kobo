#[derive(Clone, Debug)]
pub(super) struct ParallelSafetyGate {
    pub(super) accepted: bool,
    pub(super) analysis_gate: String,
    pub(super) checks: Vec<String>,
    pub(super) blockers: Vec<String>,
}

use super::receivers::{
    block_mentions_ward_boundary, block_mutates_binding_name, iterator_source_ident,
};
use super::spawn_captures::collect_block_captures;
use super::*;

impl<'a> Lowerer<'a> {
    pub(super) fn parallel_loop_evidence(
        &self,
        for_loop: &syn::ExprForLoop,
        scopes: &ScopeStack,
        source_line: usize,
        lowering: &str,
        policy: &str,
        safety_gate: ParallelSafetyGate,
    ) -> ParallelLoopEvidence {
        let iterator = for_loop.expr.to_token_stream().to_string();
        let captured_bindings = collect_block_captures(&for_loop.body, scopes)
            .into_iter()
            .map(|binding| format!("{}:{:?}", binding.name, binding.tier))
            .collect::<Vec<_>>();
        let proof = format!(
            "{} gate={} iterator={} captures=[{}] checks=[{}]",
            lowering,
            safety_gate.analysis_gate,
            iterator,
            captured_bindings.join(","),
            safety_gate.checks.join(",")
        );
        ParallelLoopEvidence {
            source_line,
            lowering: lowering.to_owned(),
            policy: policy.to_owned(),
            analysis_gate: safety_gate.analysis_gate,
            proof,
            iterator,
            captured_bindings,
            safety_checks: safety_gate.checks,
        }
    }

    pub(super) fn parallel_safety_gate(
        &self,
        for_loop: &syn::ExprForLoop,
        scopes: &ScopeStack,
        has_explicit_policy: bool,
    ) -> ParallelSafetyGate {
        let captured = collect_block_captures(&for_loop.body, scopes);
        let captured_names = captured
            .iter()
            .map(|binding| binding.name.as_str())
            .collect::<Vec<_>>();
        let iterator_source = iterator_source_ident(for_loop.expr.as_ref())
            .map(|ident| ident.to_string())
            .unwrap_or_default();
        let mut blockers = Vec::new();

        for binding in &captured {
            if matches!(
                binding.tier,
                kobo_ir::OwnershipTier::RcShared | kobo_ir::OwnershipTier::RcMutShared
            ) {
                blockers.push(format!("non-send-capture:{}", binding.name));
            }
            if block_mutates_binding_name(&for_loop.body, &binding.name) {
                blockers.push(format!("shared-mutation:{}", binding.name));
            }
        }

        for warning in kobo_analysis::scan_source_parallel_warnings(self.ast.source()) {
            match warning.kind {
                kobo_analysis::ParallelWarningKind::NonSendCapture { binding_name, .. }
                    if captured_names
                        .iter()
                        .any(|name| *name == binding_name.as_str())
                        || binding_name == iterator_source =>
                {
                    blockers.push(format!("non-send-capture:{binding_name}"));
                }
                kobo_analysis::ParallelWarningKind::SharedMutation { binding_name }
                    if captured_names
                        .iter()
                        .any(|name| *name == binding_name.as_str()) =>
                {
                    blockers.push(format!("shared-mutation:{binding_name}"));
                }
                kobo_analysis::ParallelWarningKind::MissingBoundaryPolicy
                    if !has_explicit_policy && block_mentions_ward_boundary(&for_loop.body) =>
                {
                    blockers.push("missing-boundary-policy".to_owned());
                }
                _ => {}
            }
        }

        if !has_explicit_policy && block_mentions_ward_boundary(&for_loop.body) {
            blockers.push("missing-boundary-policy".to_owned());
        }

        blockers.sort();
        blockers.dedup();

        if blockers.is_empty() {
            ParallelSafetyGate {
                accepted: true,
                analysis_gate: "accepted-lowering-gate:no-K0061-blockers".to_owned(),
                checks: vec![
                    "accepted-lowering-gate".to_owned(),
                    "analysis-diagnostics-clean".to_owned(),
                    "no-K0061-blockers".to_owned(),
                    "send-sync".to_owned(),
                    "shared-mutation-rejected".to_owned(),
                    "ward-boundary-policy-checked".to_owned(),
                    "ast-method-call-lowered".to_owned(),
                ],
                blockers,
            }
        } else {
            ParallelSafetyGate {
                accepted: false,
                analysis_gate: format!("blocked-lowering-gate:{}", blockers.join("|")),
                checks: blockers
                    .iter()
                    .map(|blocker| format!("blocked:{blocker}"))
                    .collect(),
                blockers,
            }
        }
    }
}

impl ParallelSafetyGate {
    pub(super) fn policy(analysis_gate: &str) -> Self {
        Self {
            accepted: false,
            analysis_gate: analysis_gate.to_owned(),
            checks: vec![analysis_gate.to_owned()],
            blockers: Vec::new(),
        }
    }
}
