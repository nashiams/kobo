use std::collections::{HashMap, HashSet};

use kobo_ir::{
    CloneElisionDecision, OwnershipHint, OwnershipTier, TierDecision, TierReason,
    TransformBindingFacts, TransformFacts, UseKind,
};

const LADDER: [OwnershipTier; 5] = [
    OwnershipTier::PlainOwned,
    OwnershipTier::BoxOwned,
    OwnershipTier::RcShared,
    OwnershipTier::ArcShared,
    OwnershipTier::RcMutShared,
];

pub(crate) fn choose_tiers(facts: &TransformFacts, kir: &mut kobo_ir::Kir) -> Vec<TierDecision> {
    let mut decisions = Vec::with_capacity(facts.bindings.len());

    for binding in facts.iter_bindings() {
        decisions.push(choose_tier_for_binding(binding));
    }

    let mut seen = HashSet::new();
    for decision in &decisions {
        assert!(
            seen.insert(decision.node),
            "choose_tiers produced two decisions for the same node"
        );
    }

    apply_decisions(kir, &decisions);
    decisions
}

fn choose_tier_for_binding(binding: &TransformBindingFacts) -> TierDecision {
    if binding.resource_kind.is_some() {
        return TierDecision {
            node: binding.node,
            tier: OwnershipTier::Scoped,
            reason: TierReason::ResourceWrapper,
            annotate: true,
        };
    }

    if binding.is_copy_known {
        return TierDecision {
            node: binding.node,
            tier: OwnershipTier::PlainOwned,
            reason: TierReason::CopyType,
            annotate: false,
        };
    }

    if binding.is_generic
        && (binding.shared_facts.needs_sharing
            || binding.shared_facts.needs_mutable_wrapper
            || binding.shared_facts.box_reason.is_some())
    {
        return TierDecision {
            node: binding.node,
            tier: OwnershipTier::RcMutShared,
            reason: TierReason::GenericWrapperFloor,
            annotate: true,
        };
    }

    let mut order = candidate_order(binding.hint);
    if order.is_empty() {
        order.extend(LADDER);
    }

    let mut chosen = None;
    for candidate in order {
        if candidate_is_valid(candidate, binding)
            && candidate_satisfies_constraints(candidate, binding)
        {
            chosen = Some(candidate);
            break;
        }
    }

    let tier = chosen.unwrap_or_else(|| fallback_tier(binding));
    let reason = reason_for_tier(binding, tier);
    TierDecision {
        node: binding.node,
        tier,
        annotate: !matches!(reason, TierReason::CopyType),
        reason,
    }
}

fn candidate_order(hint: Option<OwnershipHint>) -> Vec<OwnershipTier> {
    match hint {
        Some(OwnershipHint::Move | OwnershipHint::Exclusive) | None => LADDER.to_vec(),
        Some(OwnershipHint::Shared) => vec![
            OwnershipTier::RcShared,
            OwnershipTier::ArcShared,
            OwnershipTier::RcMutShared,
            OwnershipTier::PlainOwned,
            OwnershipTier::BoxOwned,
        ],
        Some(OwnershipHint::Async) => vec![
            OwnershipTier::ArcShared,
            OwnershipTier::RcShared,
            OwnershipTier::RcMutShared,
            OwnershipTier::PlainOwned,
            OwnershipTier::BoxOwned,
        ],
    }
}

fn candidate_is_valid(candidate: OwnershipTier, binding: &TransformBindingFacts) -> bool {
    match candidate {
        OwnershipTier::PlainOwned => true,
        OwnershipTier::BoxOwned => binding.shared_facts.box_reason.is_some() && !binding.is_async,
        OwnershipTier::RcShared => true,
        OwnershipTier::ArcShared => binding.shared_facts.needs_send,
        OwnershipTier::RcMutShared => true,
        OwnershipTier::ArcMutShared => false,
        OwnershipTier::Scoped => false,
        OwnershipTier::Undecided => false,
    }
}

fn candidate_satisfies_constraints(
    candidate: OwnershipTier,
    binding: &TransformBindingFacts,
) -> bool {
    if binding.shared_facts.needs_mutable_wrapper {
        return matches!(candidate, OwnershipTier::RcMutShared);
    }

    if binding.shared_facts.needs_send {
        return matches!(
            candidate,
            OwnershipTier::ArcShared | OwnershipTier::RcMutShared
        );
    }

    if binding.shared_facts.needs_sharing {
        return matches!(
            candidate,
            OwnershipTier::RcShared | OwnershipTier::ArcShared | OwnershipTier::RcMutShared
        );
    }

    if binding.is_async && binding.shared_facts.box_reason.is_some() {
        return matches!(
            candidate,
            OwnershipTier::RcShared | OwnershipTier::ArcShared | OwnershipTier::RcMutShared
        );
    }

    if binding.shared_facts.box_reason.is_some() {
        return matches!(
            candidate,
            OwnershipTier::BoxOwned | OwnershipTier::RcShared | OwnershipTier::RcMutShared
        );
    }

    true
}

fn fallback_tier(binding: &TransformBindingFacts) -> OwnershipTier {
    if binding.shared_facts.needs_mutable_wrapper {
        OwnershipTier::RcMutShared
    } else if binding.shared_facts.needs_send {
        OwnershipTier::ArcShared
    } else if binding.shared_facts.needs_sharing {
        OwnershipTier::RcShared
    } else if binding.is_async && binding.shared_facts.box_reason.is_some() {
        OwnershipTier::RcShared
    } else if binding.shared_facts.box_reason.is_some() && !binding.is_async {
        OwnershipTier::BoxOwned
    } else {
        OwnershipTier::PlainOwned
    }
}

fn reason_for_tier(binding: &TransformBindingFacts, tier: OwnershipTier) -> TierReason {
    match tier {
        OwnershipTier::RcShared
            if binding.is_async && binding.shared_facts.box_reason.is_some() =>
        {
            TierReason::AsyncBoxDeferred
        }
        OwnershipTier::PlainOwned
            if binding.is_async && binding.shared_facts.box_reason.is_some() =>
        {
            TierReason::AsyncBoxDeferred
        }
        OwnershipTier::PlainOwned if binding.clone_elision == Some(CloneElisionDecision::Move) => {
            TierReason::DeadOriginalAfterAssignment
        }
        OwnershipTier::PlainOwned => TierReason::LocalOnly,
        OwnershipTier::BoxOwned => TierReason::HeapStable(
            binding
                .shared_facts
                .box_reason
                .unwrap_or(kobo_ir::BoxReason::StackSizeHeuristic),
        ),
        OwnershipTier::RcShared => TierReason::ReadOnlyShared {
            read_sites: binding.shared_facts.read_sites.max(2),
            sequential_only: binding.shared_facts.sequential_read_only,
        },
        OwnershipTier::ArcShared => TierReason::SendRequiredShared,
        OwnershipTier::RcMutShared => TierReason::MutableSharedLastResort,
        OwnershipTier::ArcMutShared => TierReason::SendRequiredShared,
        OwnershipTier::Scoped => TierReason::ResourceWrapper,
        OwnershipTier::Undecided => TierReason::LocalOnly,
    }
}

pub(crate) fn apply_decisions(kir: &mut kobo_ir::Kir, decisions: &[TierDecision]) {
    let decisions_by_node = decisions
        .iter()
        .map(|decision| (decision.node, decision))
        .collect::<HashMap<_, _>>();

    for node in kir.iter_nodes_mut() {
        if let Some(decision) = decisions_by_node.get(&node.id) {
            node.ownership = decision.tier;
        }

        if let Some(decl_id) = node.decl_id {
            if let Some(decision) = decisions_by_node.get(&decl_id) {
                node.ownership = decision.tier;
                match (node.kind, decision.tier) {
                    (kobo_ir::NodeKind::Move, tier) if tier.is_cloneable_wrapper() => {
                        node.kind = kobo_ir::NodeKind::Use(UseKind::Read);
                    }
                    (
                        kobo_ir::NodeKind::Borrow(kobo_ir::BorrowKind::Immutable),
                        OwnershipTier::RcShared | OwnershipTier::ArcShared,
                    ) => {
                        node.kind = kobo_ir::NodeKind::Use(UseKind::Read);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use kobo_ir::{
        BindingUsage, BoxReason, FileId, KoboAstNodeId, KoboSpan, OwnershipTier,
        SharedBindingFacts, TierReason, TransformBindingFacts,
    };

    use super::choose_tier_for_binding;

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
        }
    }

    #[test]
    fn async_box_sites_fall_back_to_rc_shared() {
        let mut async_binding = binding(SharedBindingFacts {
            box_reason: Some(BoxReason::RecursiveType),
            ..Default::default()
        });
        async_binding.is_async = true;

        let decision = choose_tier_for_binding(&async_binding);

        assert_eq!(decision.tier, OwnershipTier::RcShared);
        assert_eq!(decision.reason, TierReason::AsyncBoxDeferred);
    }
}
