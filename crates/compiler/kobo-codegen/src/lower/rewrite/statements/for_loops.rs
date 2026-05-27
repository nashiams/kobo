use super::super::parallel_gate::ParallelSafetyGate;
use super::super::{parallel, Lowerer, ScopeStack};

impl<'a> Lowerer<'a> {
    pub(super) fn lower_for_loop_statement(
        &mut self,
        for_loop: &mut syn::ExprForLoop,
        scopes: &mut ScopeStack,
    ) -> Option<syn::Expr> {
        let policy =
            parallel::policy_value(&for_loop.attrs).unwrap_or_else(|| "outside".to_owned());
        let has_explicit_policy = parallel::policy_value(&for_loop.attrs).is_some();
        let source_line = self.source_line_for_expr_for_loop(for_loop);
        let safety_gate = self.parallel_safety_gate(for_loop, scopes, has_explicit_policy);

        match parallel::lower_for_loop(for_loop, safety_gate.accepted) {
            parallel::ParallelLowering::Parallel => Some(self.parallel_for_each_statement(
                for_loop,
                scopes,
                source_line,
                &policy,
                has_explicit_policy,
                safety_gate,
            )),
            parallel::ParallelLowering::SerialPolicy => {
                parallel::mark_serial_policy(for_loop);
                self.push_parallel_statement_evidence(
                    for_loop,
                    scopes,
                    source_line,
                    "serial",
                    "serial-order",
                    ParallelSafetyGate::policy("policy-gate:serial-order"),
                );
                None
            }
            parallel::ParallelLowering::BoundaryPolicy => {
                parallel::mark_boundary_policy(for_loop, &policy);
                self.push_parallel_statement_evidence(
                    for_loop,
                    scopes,
                    source_line,
                    "boundary-policy",
                    &policy,
                    ParallelSafetyGate::policy("policy-gate:ward-boundary"),
                );
                None
            }
            parallel::ParallelLowering::SafetyBlocked => {
                parallel::mark_safety_blocked(for_loop, &safety_gate.blockers);
                self.push_parallel_statement_evidence(
                    for_loop,
                    scopes,
                    source_line,
                    "blocked-safety",
                    "safety-gate",
                    safety_gate,
                );
                None
            }
            parallel::ParallelLowering::None => None,
        }
    }

    fn parallel_for_each_statement(
        &mut self,
        for_loop: &mut syn::ExprForLoop,
        scopes: &mut ScopeStack,
        source_line: usize,
        policy: &str,
        has_explicit_policy: bool,
        safety_gate: ParallelSafetyGate,
    ) -> syn::Expr {
        self.needs_rayon = true;
        if has_explicit_policy {
            parallel::mark_boundary_policy(for_loop, policy);
        }
        self.lower_expr(for_loop.expr.as_mut(), scopes);
        self.lower_nested_block(&mut for_loop.body, scopes);
        self.push_parallel_statement_evidence(
            for_loop,
            scopes,
            source_line,
            "rayon-par-iter",
            policy,
            safety_gate,
        );
        parallel::for_each_adapter_expr(for_loop)
    }

    fn push_parallel_statement_evidence(
        &mut self,
        for_loop: &syn::ExprForLoop,
        scopes: &ScopeStack,
        source_line: usize,
        lowering: &str,
        policy: &str,
        safety_gate: ParallelSafetyGate,
    ) {
        let evidence = self.parallel_loop_evidence(
            for_loop,
            scopes,
            source_line,
            lowering,
            policy,
            safety_gate,
        );
        self.parallel_evidence.push(evidence);
    }
}
