use std::collections::{HashMap, HashSet};

use kobo_ir::{
    CloneElisionDecision, EscapeKind, OwnershipHint, OwnershipTier, SatisfactionCheck,
    SharedBindingFacts, TierDecision, TierReason, TransformBindingFacts, TransformFacts, UseEvent,
    UseKind,
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
    let floor = max_floor(&binding.shared_facts);

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

    if binding.is_generic && floor != OwnershipTier::PlainOwned {
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
            && candidate_satisfies_constraints(candidate, binding, floor)
        {
            chosen = Some(candidate);
            break;
        }
    }

    let tier = chosen.unwrap_or_else(|| fallback_tier(binding, floor));
    let reason = reason_for_tier(binding, tier);
    TierDecision {
        node: binding.node,
        tier,
        annotate: !matches!(reason, TierReason::CopyType),
        reason,
    }
}

#[allow(dead_code)]
fn floor_of_event(event: &UseEvent) -> u8 {
    match event {
        UseEvent::Mutated { .. }
        | UseEvent::Borrowed {
            kind: kobo_ir::BorrowKind::Mutable,
            ..
        } => OwnershipTier::RcMutShared.priority() as u8,
        UseEvent::Escaped { .. } => OwnershipTier::BoxOwned.priority() as u8,
        UseEvent::ReadOnly { .. }
        | UseEvent::Borrowed {
            kind: kobo_ir::BorrowKind::Immutable,
            ..
        } => OwnershipTier::RcShared.priority() as u8,
        UseEvent::Moved { .. } => OwnershipTier::PlainOwned.priority() as u8,
    }
}

#[allow(dead_code)]
fn max_floor(facts: &SharedBindingFacts) -> OwnershipTier {
    if facts.mutation_required {
        return OwnershipTier::RcMutShared;
    }
    if facts.escape_floor.is_some() || facts.box_reason.is_some() {
        return OwnershipTier::BoxOwned;
    }
    if facts.needs_send {
        return OwnershipTier::ArcShared;
    }
    if facts.needs_sharing || facts.live_borrow_at_move {
        return OwnershipTier::RcShared;
    }
    OwnershipTier::PlainOwned
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
    floor: OwnershipTier,
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

    match floor {
        OwnershipTier::PlainOwned => true,
        OwnershipTier::RcShared => matches!(
            candidate,
            OwnershipTier::RcShared | OwnershipTier::ArcShared | OwnershipTier::RcMutShared
        ),
        OwnershipTier::ArcShared => matches!(candidate, OwnershipTier::ArcShared),
        OwnershipTier::BoxOwned
            if binding.shared_facts.escape_floor == Some(EscapeKind::PassedToOpaqueCall) =>
        {
            matches!(candidate, OwnershipTier::BoxOwned)
        }
        OwnershipTier::BoxOwned => matches!(
            candidate,
            OwnershipTier::BoxOwned | OwnershipTier::RcShared | OwnershipTier::RcMutShared
        ),
        OwnershipTier::RcMutShared => matches!(candidate, OwnershipTier::RcMutShared),
        OwnershipTier::ArcMutShared => matches!(candidate, OwnershipTier::ArcMutShared),
        OwnershipTier::Scoped | OwnershipTier::Undecided => false,
    }
}

fn fallback_tier(binding: &TransformBindingFacts, floor: OwnershipTier) -> OwnershipTier {
    if binding.shared_facts.needs_send {
        return OwnershipTier::ArcShared;
    }
    if binding.shared_facts.needs_mutable_wrapper {
        return OwnershipTier::RcMutShared;
    }
    if matches!(floor, OwnershipTier::BoxOwned) {
        return if binding.is_async {
            OwnershipTier::RcShared
        } else if binding.shared_facts.box_reason.is_some() {
            OwnershipTier::BoxOwned
        } else {
            OwnershipTier::PlainOwned
        };
    }
    floor
}

fn reason_for_tier(binding: &TransformBindingFacts, tier: OwnershipTier) -> TierReason {
    match tier {
        OwnershipTier::RcShared
            if binding.shared_facts.escape_floor == Some(EscapeKind::ReturnedFromFunction) =>
        {
            TierReason::ValidationEscalation(SatisfactionCheck::ReturnEscapeBoxDeferred)
        }
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
        BindingUsage, BoxReason, EscapeKind, FileId, KoboAstNodeId, KoboSpan, OwnershipTier,
        SharedBindingFacts, TierReason, TransformBindingFacts, UseEvent,
    };

    use super::{choose_tier_for_binding, floor_of_event, max_floor};

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

    #[test]
    fn floor_of_event_assigns_box_priority_to_escape_events() {
        let event = UseEvent::Escaped {
            kind: EscapeKind::ReturnedFromFunction,
            span: KoboSpan::new(0, 1, FileId(0)),
        };

        assert_eq!(
            floor_of_event(&event),
            OwnershipTier::BoxOwned.priority() as u8
        );
    }

    #[test]
    fn max_floor_prefers_escape_over_read_only_sharing() {
        let floor = max_floor(&SharedBindingFacts {
            needs_sharing: true,
            escape_floor: Some(EscapeKind::ReturnedFromFunction),
            ..Default::default()
        });

        assert_eq!(floor, OwnershipTier::BoxOwned);
    }

    #[test]
    fn max_floor_for_immutable_borrow_and_return_escape_is_box_owned() {
        let floor = max_floor(&SharedBindingFacts {
            needs_sharing: true,
            borrow_sites: vec![KoboSpan::new(0, 1, FileId(0))],
            escape_floor: Some(EscapeKind::ReturnedFromFunction),
            ..Default::default()
        });

        assert_eq!(floor, OwnershipTier::BoxOwned);
    }

    #[test]
    fn max_floor_prefers_mutation_over_escape() {
        let floor = max_floor(&SharedBindingFacts {
            mutation_required: true,
            needs_mutable_wrapper: true,
            escape_floor: Some(EscapeKind::StoredInStruct),
            ..Default::default()
        });

        assert_eq!(floor, OwnershipTier::RcMutShared);
    }

    #[test]
    fn max_floor_for_mutable_borrow_and_struct_escape_is_rc_refcell() {
        let floor = max_floor(&SharedBindingFacts {
            mutation_required: true,
            needs_mutable_wrapper: true,
            borrow_sites: vec![KoboSpan::new(0, 1, FileId(0))],
            escape_floor: Some(EscapeKind::StoredInStruct),
            ..Default::default()
        });

        assert_eq!(floor, OwnershipTier::RcMutShared);
    }

    #[test]
    fn max_floor_returns_rc_shared_for_plain_read_only_sharing() {
        let floor = max_floor(&SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            ..Default::default()
        });

        assert_eq!(floor, OwnershipTier::RcShared);
    }

    #[test]
    fn max_floor_returns_highest_priority_for_three_conflicting_constraints() {
        let floor = max_floor(&SharedBindingFacts {
            needs_sharing: true,
            mutation_required: true,
            needs_mutable_wrapper: true,
            escape_floor: Some(EscapeKind::StoredInStruct),
            ..Default::default()
        });

        assert_eq!(floor, OwnershipTier::RcMutShared);
    }

    #[test]
    fn local_only_bindings_choose_plain_owned() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts::default()));

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::LocalOnly);
    }

    #[test]
    fn box_reason_chooses_box_owned_before_shared_wrappers() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            box_reason: Some(BoxReason::StackSizeHeuristic),
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::BoxOwned);
    }

    #[test]
    fn return_escape_chooses_rc_shared_with_deferred_reason() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            escape_floor: Some(EscapeKind::ReturnedFromFunction),
            has_escape: true,
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcShared);
        assert_eq!(
            decision.reason,
            TierReason::ValidationEscalation(kobo_ir::SatisfactionCheck::ReturnEscapeBoxDeferred)
        );
    }

    #[test]
    fn read_only_sharing_chooses_rc_shared() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcShared);
    }

    #[test]
    fn opaque_call_escape_stays_plain_owned_until_boundary_rewrite_exists() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            escape_floor: Some(EscapeKind::PassedToOpaqueCall),
            has_escape: true,
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::LocalOnly);
    }

    #[test]
    fn send_required_sharing_chooses_arc_shared() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            needs_send: true,
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::ArcShared);
    }

    #[test]
    fn mutable_floor_chooses_rc_refcell() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            mutation_required: true,
            needs_mutable_wrapper: true,
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn mutable_borrow_only_still_chooses_rc_refcell() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            mutation_required: true,
            needs_mutable_wrapper: true,
            borrow_sites: vec![KoboSpan::new(0, 1, FileId(0))],
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn two_immutable_borrows_choose_rc_shared() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            needs_sharing: true,
            borrow_sites: vec![
                KoboSpan::new(0, 1, FileId(0)),
                KoboSpan::new(2, 3, FileId(0)),
            ],
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcShared);
    }

    #[test]
    fn immutable_and_mutable_borrow_choose_rc_refcell() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            needs_sharing: true,
            mutation_required: true,
            needs_mutable_wrapper: true,
            borrow_sites: vec![
                KoboSpan::new(0, 1, FileId(0)),
                KoboSpan::new(2, 3, FileId(0)),
            ],
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn single_immutable_borrow_stays_plain_owned() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            borrow_sites: vec![KoboSpan::new(0, 1, FileId(0))],
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    }

    #[test]
    fn read_only_then_mutable_prefers_rc_refcell() {
        let decision = choose_tier_for_binding(&binding(SharedBindingFacts {
            needs_sharing: true,
            read_sites: 1,
            mutation_required: true,
            mutable_sites: 1,
            needs_mutable_wrapper: true,
            ..Default::default()
        }));

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn generic_local_only_binding_stays_plain_owned() {
        let mut generic_binding = binding(SharedBindingFacts::default());
        generic_binding.is_generic = true;

        let decision = choose_tier_for_binding(&generic_binding);

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    }

    #[test]
    fn generic_wrapper_floor_uses_rc_refcell() {
        let mut generic_binding = binding(SharedBindingFacts {
            needs_sharing: true,
            ..Default::default()
        });
        generic_binding.is_generic = true;

        let decision = choose_tier_for_binding(&generic_binding);

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
        assert_eq!(decision.reason, TierReason::GenericWrapperFloor);
    }

    #[test]
    fn structurally_equal_shared_facts_choose_identical_tiers() {
        let shared_facts = SharedBindingFacts {
            needs_sharing: true,
            read_sites: 2,
            ..Default::default()
        };
        let left = choose_tier_for_binding(&binding(shared_facts.clone()));
        let mut right_binding = binding(shared_facts);
        right_binding.binding_name = "other".to_owned();
        let right = choose_tier_for_binding(&right_binding);

        assert_eq!(left.tier, right.tier);
    }

    #[test]
    fn async_box_sites_fall_back_to_rc_shared() {
        let mut async_binding = binding(SharedBindingFacts {
            box_reason: Some(BoxReason::StackSizeHeuristic),
            ..Default::default()
        });
        async_binding.is_async = true;

        let decision = choose_tier_for_binding(&async_binding);

        assert_eq!(decision.tier, OwnershipTier::RcShared);
        assert_eq!(decision.reason, TierReason::AsyncBoxDeferred);
    }
}
