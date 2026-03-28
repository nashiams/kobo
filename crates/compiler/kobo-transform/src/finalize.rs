use std::collections::HashMap;

use crate::builder::{BuilderOutput, TransformFactsBuilder};
use crate::escape::{finalize_transform_facts, BorrowAlias};
use kobo_ir::{
    HintConflictFact, HintConflictReason, Kir, NodeKind, OwnershipHint, OwnershipTier,
    SatisfactionCheck, TierDecision, TierReason, TierViolation, TransformBindingFacts,
    TransformFacts, UseEvent, UseKind,
};

pub(crate) fn finalize_transform(builder: TransformFactsBuilder<'_>) -> BuilderOutput {
    let mut output = builder.finish();
    finalize_transform_facts(&mut output.transform_facts, &output.borrow_aliases);
    output
}

pub(crate) fn collect_hint_conflicts(facts: &mut TransformFacts, decisions: &[TierDecision]) {
    let decisions_by_node = decisions
        .iter()
        .map(|decision| (decision.node, decision))
        .collect::<HashMap<_, _>>();

    for binding in &facts.bindings {
        let Some(hint) = binding.hint else {
            continue;
        };
        let Some(hint_span) = binding.hint_span else {
            continue;
        };
        let Some(decision) = decisions_by_node.get(&binding.node) else {
            continue;
        };
        if hint_matches_tier(hint, decision.tier) {
            continue;
        }

        let conflict_span = binding
            .usage
            .uses
            .first()
            .map(UseEvent::span)
            .unwrap_or(binding.span);
        facts.hint_conflicts.push(HintConflictFact {
            node: binding.node,
            hint,
            hint_span,
            conflict_span,
            chosen_tier: decision.tier,
            reason: hint_conflict_reason(binding),
        });
    }
}

pub(crate) fn rewrite_dead_borrow_aliases(
    kir: &mut Kir,
    transform_facts: &TransformFacts,
    borrow_aliases: &[BorrowAlias],
) {
    for alias in borrow_aliases {
        let alias_is_dead = transform_facts
            .binding(alias.alias)
            .is_some_and(|binding| binding.usage.uses.is_empty());
        if !alias_is_dead {
            continue;
        }

        for node in kir.iter_nodes_mut() {
            if node.decl_id != Some(alias.source) {
                continue;
            }
            if node.span != alias.span {
                continue;
            }
            if node.kind == NodeKind::Borrow(alias.kind) {
                node.kind = NodeKind::Use(UseKind::Read);
            }
        }
    }
}

pub(crate) fn apply_validation_escalations(
    decisions: &mut [TierDecision],
    violations: &[TierViolation],
) {
    let mut escalations = HashMap::<kobo_ir::KirNodeId, (OwnershipTier, SatisfactionCheck)>::new();

    for violation in violations {
        let escalation = escalations
            .entry(violation.node_id)
            .or_insert((violation.escalated_to, violation.failed_check));
        if violation.escalated_to.priority() > escalation.0.priority() {
            *escalation = (violation.escalated_to, violation.failed_check);
        }
    }

    for decision in decisions {
        let Some((tier, check)) = escalations.get(&decision.node).copied() else {
            continue;
        };
        decision.tier = tier;
        decision.reason = TierReason::ValidationEscalation(check);
        decision.annotate = true;
    }
}

fn hint_matches_tier(hint: OwnershipHint, tier: OwnershipTier) -> bool {
    match hint {
        OwnershipHint::Move | OwnershipHint::Exclusive => {
            matches!(tier, OwnershipTier::PlainOwned | OwnershipTier::BoxOwned)
        }
        OwnershipHint::Shared => tier.is_shared(),
        OwnershipHint::Async => {
            matches!(tier, OwnershipTier::ArcShared | OwnershipTier::ArcMutShared)
        }
    }
}

fn hint_conflict_reason(binding: &TransformBindingFacts) -> HintConflictReason {
    if binding.shared_facts.needs_mutable_wrapper {
        HintConflictReason::MutableUse
    } else if binding.shared_facts.needs_send {
        HintConflictReason::SendRequired
    } else if binding.shared_facts.live_borrow_at_move {
        HintConflictReason::Aliasing
    } else if binding.shared_facts.needs_sharing {
        HintConflictReason::SharedUsage
    } else if binding.is_async && binding.shared_facts.box_reason.is_some() {
        HintConflictReason::AsyncDeferred
    } else {
        HintConflictReason::Unknown
    }
}
