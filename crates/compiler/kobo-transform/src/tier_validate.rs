use kobo_ir::{
    EscapeKind, OwnershipTier, SatisfactionCheck, TierDecision, TierViolation, TransformFacts,
};

pub(crate) fn validate_tiers(
    decisions: &[TierDecision],
    facts: &TransformFacts,
) -> Vec<TierViolation> {
    let mut violations = Vec::new();

    for decision in decisions {
        let Some(binding) = facts.binding(decision.node) else {
            continue;
        };
        if binding.is_copy_known || binding.resource_kind.is_some() {
            continue;
        }

        push_mutation_violation(&mut violations, decision, binding);
        push_escape_violation(&mut violations, decision, binding);
        push_sharing_violation(&mut violations, decision, binding);
        push_live_borrow_violation(&mut violations, decision, binding);
        push_async_box_violation(&mut violations, decision, binding);
    }

    violations
}

fn push_mutation_violation(
    violations: &mut Vec<TierViolation>,
    decision: &TierDecision,
    binding: &kobo_ir::TransformBindingFacts,
) {
    if binding.shared_facts.needs_mutable_wrapper && decision.tier != OwnershipTier::RcMutShared {
        violations.push(TierViolation {
            node_id: decision.node,
            chosen_tier: decision.tier,
            failed_check: SatisfactionCheck::MutationRequiredButTierIsReadOnly,
            escalated_to: OwnershipTier::RcMutShared,
        });
    }
}

fn push_escape_violation(
    violations: &mut Vec<TierViolation>,
    decision: &TierDecision,
    binding: &kobo_ir::TransformBindingFacts,
) {
    // Known Limitation 6: ReturnedFromFunction cannot produce Box<T> without rewriting
    // the function's return type and all call sites. Escalate to RcShared instead.
    // The annotation text "return escape: Box<T> requires signature rewrite (v0.4)"
    // makes the gap visible. The correct fix is v0.4 call-graph rewriting.
    if binding.shared_facts.escape_floor == Some(EscapeKind::ReturnedFromFunction)
        && decision.tier == OwnershipTier::PlainOwned
    {
        violations.push(TierViolation {
            node_id: decision.node,
            chosen_tier: decision.tier,
            failed_check: SatisfactionCheck::ReturnEscapeBoxDeferred,
            escalated_to: OwnershipTier::RcShared,
        });
        return;
    }

    // Other declaration-local box reasons remain safe to box at the binding site
    // without touching the function signature.
    if binding.shared_facts.box_reason.is_some() && decision.tier == OwnershipTier::PlainOwned {
        violations.push(TierViolation {
            node_id: decision.node,
            chosen_tier: decision.tier,
            failed_check: SatisfactionCheck::EscapeRequiresHeapButTierIsStack,
            escalated_to: OwnershipTier::BoxOwned,
        });
    }
}

fn push_sharing_violation(
    violations: &mut Vec<TierViolation>,
    decision: &TierDecision,
    binding: &kobo_ir::TransformBindingFacts,
) {
    let tier_allows_sharing = matches!(
        decision.tier,
        OwnershipTier::RcShared | OwnershipTier::ArcShared | OwnershipTier::RcMutShared
    );

    if binding.shared_facts.needs_sharing && !tier_allows_sharing {
        violations.push(TierViolation {
            node_id: decision.node,
            chosen_tier: decision.tier,
            failed_check: SatisfactionCheck::SharingRequiredButTierIsExclusive,
            escalated_to: OwnershipTier::RcShared,
        });
    }
}

fn push_live_borrow_violation(
    violations: &mut Vec<TierViolation>,
    decision: &TierDecision,
    binding: &kobo_ir::TransformBindingFacts,
) {
    if binding.shared_facts.live_borrow_at_move && decision.tier == OwnershipTier::PlainOwned {
        violations.push(TierViolation {
            node_id: decision.node,
            chosen_tier: decision.tier,
            failed_check: SatisfactionCheck::LiveBorrowAtMoveButTierAllowsMove,
            escalated_to: OwnershipTier::RcShared,
        });
    }
}

fn push_async_box_violation(
    violations: &mut Vec<TierViolation>,
    decision: &TierDecision,
    binding: &kobo_ir::TransformBindingFacts,
) {
    if binding.is_async && decision.tier == OwnershipTier::BoxOwned {
        violations.push(TierViolation {
            node_id: decision.node,
            chosen_tier: decision.tier,
            failed_check: SatisfactionCheck::AsyncBoxProhibited,
            escalated_to: OwnershipTier::RcShared,
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::finalize::apply_validation_escalations;
    use kobo_ir::{
        BindingUsage, BoxReason, FileId, KoboAstNodeId, KoboSpan, OwnershipTier,
        SharedBindingFacts, TierDecision, TierReason, TransformBindingFacts, TransformFacts,
    };

    use super::validate_tiers;

    fn binding(shared_facts: SharedBindingFacts) -> TransformBindingFacts {
        TransformBindingFacts {
            node: kobo_ir::KirNodeId(1),
            ast_id: KoboAstNodeId(1),
            binding_name: "value".to_owned(),
            span: KoboSpan::new(0, 5, FileId(0)),
            resource_kind: None,
            hint: None,
            hint_span: None,
            is_copy_known: false,
            is_generic: false,
            is_async: false,
            usage: BindingUsage::new(KoboSpan::new(0, 5, FileId(0))),
            shared_facts,
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
        }
    }

    fn decision(tier: OwnershipTier) -> TierDecision {
        TierDecision {
            node: kobo_ir::KirNodeId(1),
            tier,
            reason: TierReason::LocalOnly,
            annotate: true,
        }
    }

    fn facts(shared_facts: SharedBindingFacts) -> TransformFacts {
        TransformFacts::from_bindings(vec![binding(shared_facts)], Vec::new())
    }

    #[test]
    fn valid_tier_produces_no_violations() {
        let facts = facts(SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            ..Default::default()
        });

        assert!(validate_tiers(&[decision(OwnershipTier::RcShared)], &facts).is_empty());
    }

    #[test]
    fn mutation_violation_escalates_to_rc_refcell() {
        let facts = facts(SharedBindingFacts {
            needs_mutable_wrapper: true,
            mutable_sites: 1,
            ..Default::default()
        });

        let violations = validate_tiers(&[decision(OwnershipTier::RcShared)], &facts);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].escalated_to, OwnershipTier::RcMutShared);
    }

    #[test]
    fn sharing_violation_escalates_to_rc_shared() {
        let facts = facts(SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            ..Default::default()
        });

        let violations = validate_tiers(&[decision(OwnershipTier::PlainOwned)], &facts);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].escalated_to, OwnershipTier::RcShared);
    }

    #[test]
    fn heap_violation_escalates_to_box_owned() {
        let facts = facts(SharedBindingFacts {
            box_reason: Some(BoxReason::StackSizeHeuristic),
            ..Default::default()
        });

        let violations = validate_tiers(&[decision(OwnershipTier::PlainOwned)], &facts);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].escalated_to, OwnershipTier::BoxOwned);
    }

    #[test]
    fn async_box_violation_escalates_to_rc_shared() {
        let mut async_binding = binding(SharedBindingFacts {
            box_reason: Some(BoxReason::StackSizeHeuristic),
            ..Default::default()
        });
        async_binding.is_async = true;
        let facts = TransformFacts::from_bindings(vec![async_binding], Vec::new());

        let violations = validate_tiers(&[decision(OwnershipTier::BoxOwned)], &facts);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].escalated_to, OwnershipTier::RcShared);
    }

    #[test]
    fn multiple_violations_return_all_failures_and_keep_strongest_escalation() {
        let facts = facts(SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            needs_mutable_wrapper: true,
            mutable_sites: 1,
            mutation_required: true,
            live_borrow_at_move: true,
            ..Default::default()
        });
        let mut decisions = vec![decision(OwnershipTier::PlainOwned)];

        let violations = validate_tiers(&decisions, &facts);

        assert_eq!(violations.len(), 3);
        apply_validation_escalations(&mut decisions, &violations);
        assert_eq!(decisions[0].tier, OwnershipTier::RcMutShared);
        assert_eq!(
            decisions[0].reason,
            TierReason::ValidationEscalation(
                kobo_ir::SatisfactionCheck::MutationRequiredButTierIsReadOnly
            )
        );
    }

    #[test]
    fn return_escape_escalates_to_rc_shared_not_box_owned() {
        // Known Limitation 6: ReturnedFromFunction cannot produce Box<T> without
        // rewriting the function's return type. The validator escalates to RcShared.
        let mut return_escape_binding = binding(SharedBindingFacts {
            escape_floor: Some(kobo_ir::EscapeKind::ReturnedFromFunction),
            has_escape: true,
            ..Default::default()
        });
        return_escape_binding
            .usage
            .push_use(kobo_ir::UseEvent::Escaped {
                kind: kobo_ir::EscapeKind::ReturnedFromFunction,
                span: KoboSpan::new(10, 15, FileId(0)),
            });
        let facts = TransformFacts::from_bindings(vec![return_escape_binding], Vec::new());

        let violations = validate_tiers(&[decision(OwnershipTier::PlainOwned)], &facts);

        assert_eq!(violations.len(), 1);
        assert_eq!(
            violations[0].failed_check,
            kobo_ir::SatisfactionCheck::ReturnEscapeBoxDeferred
        );
        assert_eq!(violations[0].escalated_to, OwnershipTier::RcShared);
    }

    #[test]
    fn rerunning_validation_after_escalation_is_empty() {
        let facts = facts(SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            needs_mutable_wrapper: true,
            mutable_sites: 1,
            mutation_required: true,
            ..Default::default()
        });
        let mut decisions = vec![decision(OwnershipTier::PlainOwned)];

        let violations = validate_tiers(&decisions, &facts);
        apply_validation_escalations(&mut decisions, &violations);

        assert!(validate_tiers(&decisions, &facts).is_empty());
    }
}
