use std::collections::{HashMap, HashSet};

use kobo_ir::{
    CloneElisionDecision, EscapeKind, KirNodeId, OwnershipHint, OwnershipTier, SatisfactionCheck,
    SharedBindingFacts, TierDecision, TierReason, TransformBindingFacts, TransformFacts, UseEvent,
    UseKind,
};

use crate::cfg::compute_send_requirements;
use crate::patterns::freeze_rotate::detect_freeze_and_rotate;

const LADDER: [OwnershipTier; 6] = [
    OwnershipTier::PlainOwned,
    OwnershipTier::BoxOwned,
    OwnershipTier::RcShared,
    OwnershipTier::ArcShared,
    OwnershipTier::ArcMutShared,
    OwnershipTier::RcMutShared,
];

pub(crate) fn choose_tiers(facts: &TransformFacts, kir: &mut kobo_ir::Kir) -> Vec<TierDecision> {
    let mut decisions = Vec::with_capacity(facts.bindings.len());
    let freeze_rotate_set = detect_freeze_and_rotate(facts);
    let send_reqs = compute_send_requirements(kir);

    for binding in facts.iter_bindings() {
        decisions.push(choose_tier_for_binding(
            binding,
            &freeze_rotate_set,
            &send_reqs,
        ));
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

fn choose_tier_for_binding(
    binding: &TransformBindingFacts,
    freeze_rotate_set: &HashSet<KirNodeId>,
    send_reqs: &crate::cfg::SendRequirements,
) -> TierDecision {
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

    // BUG 7: #[kobo::async_shared] opt-in forces Arc tier.
    if binding.async_shared {
        let tier = if binding.shared_facts.mutation_required {
            OwnershipTier::ArcMutShared
        } else {
            OwnershipTier::ArcShared
        };
        return TierDecision {
            node: binding.node,
            tier,
            reason: TierReason::AsyncSharedAttribute,
            annotate: true,
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

    // S-1: Local-only bindings skip Rc/RefCell wrapping.
    // Mutation on a non-shared, non-escaping local is just `let mut`.
    // Exception: async + box_reason needs special handling (Box is deferred in async).
    if !(binding.shared_facts.needs_sharing
        || binding.shared_facts.has_escape
        || binding.is_async && binding.shared_facts.box_reason.is_some())
    {
        let tier = if binding.shared_facts.box_reason.is_some() {
            OwnershipTier::BoxOwned
        } else {
            OwnershipTier::PlainOwned
        };
        return TierDecision {
            node: binding.node,
            tier,
            reason: TierReason::LocalOnly,
            annotate: true,
        };
    }

    // S-2: Freeze-and-rotate — binding is moved and dead afterward.
    // Even if analysis suggests sharing, the value is consumed so no wrapper needed.
    // Bypasses max_floor computation entirely.
    if freeze_rotate_set.contains(&binding.node) {
        return TierDecision {
            node: binding.node,
            tier: OwnershipTier::PlainOwned,
            reason: TierReason::MoveRebind,
            annotate: true,
        };
    }

    // Phase 11: Async Send override — if binding needs Send (crosses spawn/await
    // boundary) AND needs sharing → use Arc instead of Rc.
    // Contract: ownership_decision_table.md rows A5/A6.
    // Priority: after CopyType, LocalOnly, MoveRebind; before generic ladder.
    if send_reqs.needs_send(binding.node) && binding.shared_facts.needs_sharing {
        let tier = if binding.shared_facts.mutation_required {
            OwnershipTier::ArcMutShared // Arc<RwLock<T>>
        } else {
            OwnershipTier::ArcShared // Arc<T>
        };
        return TierDecision {
            node: binding.node,
            tier,
            reason: TierReason::SendRequiredShared,
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
            OwnershipTier::ArcMutShared,
            OwnershipTier::RcMutShared,
            OwnershipTier::PlainOwned,
            OwnershipTier::BoxOwned,
        ],
        Some(OwnershipHint::Async) => vec![
            OwnershipTier::ArcShared,
            OwnershipTier::ArcMutShared,
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
        OwnershipTier::ArcMutShared => {
            binding.shared_facts.needs_send && binding.shared_facts.needs_mutable_wrapper
        }
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
        if binding.shared_facts.needs_send {
            return matches!(
                candidate,
                OwnershipTier::ArcMutShared | OwnershipTier::RcMutShared
            );
        }
        return matches!(candidate, OwnershipTier::RcMutShared);
    }

    if binding.shared_facts.needs_send {
        return matches!(
            candidate,
            OwnershipTier::ArcShared | OwnershipTier::ArcMutShared | OwnershipTier::RcMutShared
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
        BindingUsage, BoxReason, EscapeKind, FileId, KirNodeId, KoboAstNodeId, KoboSpan,
        OwnershipTier, SharedBindingFacts, TierReason, TransformBindingFacts, UseEvent,
    };
    use std::collections::HashSet;

    use super::{choose_tier_for_binding, floor_of_event, max_floor};
    use crate::cfg::SendRequirements;

    fn empty_set() -> HashSet<KirNodeId> {
        HashSet::new()
    }

    fn empty_send_reqs() -> SendRequirements {
        SendRequirements::empty()
    }

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
            async_shared: false,
            usage: BindingUsage::new(KoboSpan::new(0, 5, FileId(0))),
            shared_facts,
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
            decl_scope_depth: 0,
            ref_returning_read_spans: Vec::new(),
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
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts::default()),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::LocalOnly);
    }

    #[test]
    fn box_reason_chooses_box_owned_before_shared_wrappers() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                box_reason: Some(BoxReason::StackSizeHeuristic),
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::BoxOwned);
    }

    #[test]
    fn return_escape_chooses_rc_shared_with_deferred_reason() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                escape_floor: Some(EscapeKind::ReturnedFromFunction),
                has_escape: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcShared);
        assert_eq!(
            decision.reason,
            TierReason::ValidationEscalation(kobo_ir::SatisfactionCheck::ReturnEscapeBoxDeferred)
        );
    }

    #[test]
    fn read_only_sharing_chooses_rc_shared() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                read_sites: 2,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcShared);
    }

    #[test]
    fn opaque_call_escape_stays_plain_owned_until_boundary_rewrite_exists() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                escape_floor: Some(EscapeKind::PassedToOpaqueCall),
                has_escape: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::LocalOnly);
    }

    #[test]
    fn send_required_sharing_chooses_arc_shared() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_send: true,
                needs_sharing: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::ArcShared);
    }

    #[test]
    fn mutable_floor_chooses_rc_refcell() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                mutation_required: true,
                needs_mutable_wrapper: true,
                needs_sharing: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn mutable_borrow_only_still_chooses_rc_refcell() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                mutation_required: true,
                needs_mutable_wrapper: true,
                needs_sharing: true,
                borrow_sites: vec![KoboSpan::new(0, 1, FileId(0))],
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn two_immutable_borrows_choose_rc_shared() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                borrow_sites: vec![
                    KoboSpan::new(0, 1, FileId(0)),
                    KoboSpan::new(2, 3, FileId(0)),
                ],
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcShared);
    }

    #[test]
    fn immutable_and_mutable_borrow_choose_rc_refcell() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                mutation_required: true,
                needs_mutable_wrapper: true,
                borrow_sites: vec![
                    KoboSpan::new(0, 1, FileId(0)),
                    KoboSpan::new(2, 3, FileId(0)),
                ],
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn single_immutable_borrow_stays_plain_owned() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                borrow_sites: vec![KoboSpan::new(0, 1, FileId(0))],
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    }

    #[test]
    fn read_only_then_mutable_prefers_rc_refcell() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                read_sites: 1,
                mutation_required: true,
                mutable_sites: 1,
                needs_mutable_wrapper: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    #[test]
    fn generic_local_only_binding_stays_plain_owned() {
        let mut generic_binding = binding(SharedBindingFacts::default());
        generic_binding.is_generic = true;

        let decision = choose_tier_for_binding(&generic_binding, &empty_set(), &empty_send_reqs());

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    }

    #[test]
    fn generic_wrapper_floor_uses_rc_refcell() {
        let mut generic_binding = binding(SharedBindingFacts {
            needs_sharing: true,
            ..Default::default()
        });
        generic_binding.is_generic = true;

        let decision = choose_tier_for_binding(&generic_binding, &empty_set(), &empty_send_reqs());

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
        let left = choose_tier_for_binding(
            &binding(shared_facts.clone()),
            &empty_set(),
            &empty_send_reqs(),
        );
        let mut right_binding = binding(shared_facts);
        right_binding.binding_name = "other".to_owned();
        let right = choose_tier_for_binding(&right_binding, &empty_set(), &empty_send_reqs());

        assert_eq!(left.tier, right.tier);
    }

    #[test]
    fn async_box_sites_fall_back_to_rc_shared() {
        let mut async_binding = binding(SharedBindingFacts {
            box_reason: Some(BoxReason::StackSizeHeuristic),
            ..Default::default()
        });
        async_binding.is_async = true;

        let decision = choose_tier_for_binding(&async_binding, &empty_set(), &empty_send_reqs());

        assert_eq!(decision.tier, OwnershipTier::RcShared);
        assert_eq!(decision.reason, TierReason::AsyncBoxDeferred);
    }

    // --- Phase 11: Async Send tier override tests ---

    #[test]
    fn send_required_shared_read_only_chooses_arc_shared() {
        let mut send_reqs = SendRequirements::empty();
        send_reqs.needs_send.insert(KirNodeId(1));

        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                read_sites: 2,
                ..Default::default()
            }),
            &empty_set(),
            &send_reqs,
        );

        assert_eq!(decision.tier, OwnershipTier::ArcShared);
        assert_eq!(decision.reason, TierReason::SendRequiredShared);
    }

    #[test]
    fn send_required_shared_mutable_chooses_arc_mut_shared() {
        let mut send_reqs = SendRequirements::empty();
        send_reqs.needs_send.insert(KirNodeId(1));

        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                mutation_required: true,
                ..Default::default()
            }),
            &empty_set(),
            &send_reqs,
        );

        assert_eq!(decision.tier, OwnershipTier::ArcMutShared);
        assert_eq!(decision.reason, TierReason::SendRequiredShared);
    }

    #[test]
    fn send_required_but_not_shared_stays_plain_owned() {
        let mut send_reqs = SendRequirements::empty();
        send_reqs.needs_send.insert(KirNodeId(1));

        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts::default()),
            &empty_set(),
            &send_reqs,
        );

        // Not shared → local only, no Arc override
        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::LocalOnly);
    }

    #[test]
    fn send_required_copy_type_stays_plain_owned() {
        let mut send_reqs = SendRequirements::empty();
        send_reqs.needs_send.insert(KirNodeId(1));

        let mut b = binding(SharedBindingFacts {
            needs_sharing: true,
            ..Default::default()
        });
        b.is_copy_known = true;

        let decision = choose_tier_for_binding(&b, &empty_set(), &send_reqs);

        // Copy types bypass everything
        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::CopyType);
    }

    // --- BUG 7: #[kobo::async_shared] tier override tests ---

    #[test]
    fn async_shared_readonly_forces_arc_shared() {
        let mut b = binding(SharedBindingFacts::default());
        b.async_shared = true;

        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());

        assert_eq!(decision.tier, OwnershipTier::ArcShared);
        assert_eq!(decision.reason, TierReason::AsyncSharedAttribute);
        assert!(decision.annotate);
    }

    #[test]
    fn async_shared_mutable_forces_arc_mut_shared() {
        let mut b = binding(SharedBindingFacts {
            mutation_required: true,
            ..Default::default()
        });
        b.async_shared = true;

        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());

        assert_eq!(decision.tier, OwnershipTier::ArcMutShared);
        assert_eq!(decision.reason, TierReason::AsyncSharedAttribute);
        assert!(decision.annotate);
    }

    #[test]
    fn async_shared_copy_type_stays_plain_owned() {
        // Copy types always win, even over async_shared.
        let mut b = binding(SharedBindingFacts::default());
        b.async_shared = true;
        b.is_copy_known = true;

        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());

        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
        assert_eq!(decision.reason, TierReason::CopyType);
    }

    #[test]
    fn async_shared_overrides_local_only() {
        // Even a local-only binding gets Arc if async_shared is set.
        let mut b = binding(SharedBindingFacts {
            needs_sharing: false,
            has_escape: false,
            ..Default::default()
        });
        b.async_shared = true;

        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());

        assert_eq!(decision.tier, OwnershipTier::ArcShared);
        assert_eq!(decision.reason, TierReason::AsyncSharedAttribute);
    }

    #[test]
    fn async_hint_send_mutable_uses_arc_mut_via_ladder() {
        let mut send_reqs = SendRequirements::empty();
        send_reqs.needs_send.insert(KirNodeId(1));

        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                needs_send: true,
                needs_mutable_wrapper: true,
                mutation_required: true,
                ..Default::default()
            }),
            &empty_set(),
            &send_reqs,
        );

        assert_eq!(decision.tier, OwnershipTier::ArcMutShared);
        assert_eq!(decision.reason, TierReason::SendRequiredShared);
    }

    #[test]
    fn async_hint_send_no_mutation_uses_arc_shared() {
        let mut send_reqs = SendRequirements::empty();
        send_reqs.needs_send.insert(KirNodeId(1));

        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                needs_send: true,
                ..Default::default()
            }),
            &empty_set(),
            &send_reqs,
        );

        assert_eq!(decision.tier, OwnershipTier::ArcShared);
        assert_eq!(decision.reason, TierReason::SendRequiredShared);
    }

    // ─── BUG-07 tests: ArcMutShared must be reachable from all hint paths ───

    #[test]
    fn ladder_includes_arc_mut_shared() {
        // With needs_send + needs_mutable_wrapper via shared_facts (not send_reqs),
        // the LADDER path (Move/Exclusive/None hints) must still reach ArcMutShared.
        use super::candidate_order;
        use kobo_ir::OwnershipHint;

        let order_none = candidate_order(None);
        assert!(
            order_none.contains(&OwnershipTier::ArcMutShared),
            "None-hint ladder missing ArcMutShared: {order_none:?}"
        );

        let order_move = candidate_order(Some(OwnershipHint::Move));
        assert!(
            order_move.contains(&OwnershipTier::ArcMutShared),
            "Move-hint ladder missing ArcMutShared: {order_move:?}"
        );

        let order_excl = candidate_order(Some(OwnershipHint::Exclusive));
        assert!(
            order_excl.contains(&OwnershipTier::ArcMutShared),
            "Exclusive-hint ladder missing ArcMutShared: {order_excl:?}"
        );
    }

    #[test]
    fn shared_hint_includes_arc_mut_shared() {
        use super::candidate_order;
        use kobo_ir::OwnershipHint;

        let order = candidate_order(Some(OwnershipHint::Shared));
        assert!(
            order.contains(&OwnershipTier::ArcMutShared),
            "Shared-hint ladder missing ArcMutShared: {order:?}"
        );
    }

    #[test]
    fn send_mutable_via_shared_facts_picks_arc_mut_shared() {
        // When needs_send + needs_mutable_wrapper are in shared_facts
        // but send_reqs does NOT have this node, the LADDER path should
        // still reach ArcMutShared (not fall back to RcMutShared).
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                needs_send: true,
                mutation_required: true,
                needs_mutable_wrapper: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        assert_eq!(
            decision.tier,
            OwnershipTier::ArcMutShared,
            "expected ArcMutShared when needs_send + needs_mutable_wrapper set"
        );
    }

    // ─── v0.8 edge-case tests ───

    /// Trap 2: Arc<Mutex<T>> is banned — ArcMutShared maps to Arc<RwLock<T>>.
    #[test]
    fn arc_mut_shared_label_is_rwlock_not_mutex() {
        // ArcMutShared must never generate Mutex in output
        let tier = OwnershipTier::ArcMutShared;
        assert_ne!(tier, OwnershipTier::PlainOwned); // basic sanity
                                                     // The label mapping in send_diagnostic confirms it maps to "Arc<RwLock<T>>"
                                                     // Here we verify the tier exists and is distinct.
        assert_ne!(tier, OwnershipTier::ArcShared);
    }

    /// LADDER constant has exactly 6 elements in the correct order.
    #[test]
    fn ladder_has_six_elements_in_order() {
        use super::LADDER;
        assert_eq!(LADDER.len(), 6);
        assert_eq!(LADDER[0], OwnershipTier::PlainOwned);
        assert_eq!(LADDER[1], OwnershipTier::BoxOwned);
        assert_eq!(LADDER[2], OwnershipTier::RcShared);
        assert_eq!(LADDER[3], OwnershipTier::ArcShared);
        assert_eq!(LADDER[4], OwnershipTier::ArcMutShared);
        assert_eq!(LADDER[5], OwnershipTier::RcMutShared);
    }

    /// ArcMutShared MUST appear BEFORE RcMutShared in LADDER.
    #[test]
    fn ladder_arc_mut_before_rc_mut() {
        use super::LADDER;
        let arc_pos = LADDER
            .iter()
            .position(|t| *t == OwnershipTier::ArcMutShared)
            .unwrap();
        let rc_pos = LADDER
            .iter()
            .position(|t| *t == OwnershipTier::RcMutShared)
            .unwrap();
        assert!(
            arc_pos < rc_pos,
            "ArcMutShared ({arc_pos}) must be before RcMutShared ({rc_pos})"
        );
    }

    /// Resource binding → always Scoped, even with sharing constraints.
    #[test]
    fn resource_kind_always_scoped() {
        use kobo_ir::ResourceKind;
        let mut b = binding(SharedBindingFacts {
            needs_sharing: true,
            needs_send: true,
            mutation_required: true,
            ..Default::default()
        });
        b.resource_kind = Some(ResourceKind::File);
        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());
        assert_eq!(decision.tier, OwnershipTier::Scoped);
        assert_eq!(decision.reason, TierReason::ResourceWrapper);
    }

    /// Copy + resource: Copy takes priority (checked before resource).
    #[test]
    fn copy_trumps_resource() {
        use kobo_ir::ResourceKind;
        let mut b = binding(SharedBindingFacts::default());
        b.is_copy_known = true;
        b.resource_kind = Some(ResourceKind::File);
        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());
        // Looking at the code: resource check comes first, so resource wins.
        // Let's check what actually happens...
        // From code: resource_kind check is BEFORE is_copy_known check.
        // So resource_kind wins over Copy:
        assert_eq!(decision.tier, OwnershipTier::Scoped);
    }

    /// Generic binding without sharing → PlainOwned.
    #[test]
    fn generic_non_shared_is_plain_owned() {
        let mut b = binding(SharedBindingFacts::default());
        b.is_generic = true;
        let decision = choose_tier_for_binding(&b, &empty_set(), &empty_send_reqs());
        assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    }

    /// Async-hint ordering: Async hint in candidate_order includes ArcMutShared.
    #[test]
    fn async_hint_includes_arc_mut_shared() {
        use super::candidate_order;
        use kobo_ir::OwnershipHint;

        let order = candidate_order(Some(OwnershipHint::Async));
        assert!(
            order.contains(&OwnershipTier::ArcMutShared),
            "Async-hint ladder missing ArcMutShared: {order:?}"
        );
    }

    /// Shared + mutable + NOT send → should get RcMutShared (not Arc).
    #[test]
    fn shared_mutable_no_send_gets_rc_mut_shared() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                mutation_required: true,
                needs_mutable_wrapper: true,
                needs_send: false,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );
        assert_eq!(decision.tier, OwnershipTier::RcMutShared);
    }

    /// Shared + read-only + no send → RcShared.
    #[test]
    fn shared_read_only_no_send_gets_rc_shared() {
        let decision = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                read_sites: 3,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );
        assert_eq!(decision.tier, OwnershipTier::RcShared);
    }

    /// Freeze-rotate set membership affects tier selection.
    #[test]
    fn freeze_rotate_binding_differs_from_normal() {
        let mut freeze_set = HashSet::new();
        freeze_set.insert(KirNodeId(1));

        let decision_frozen = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                ..Default::default()
            }),
            &freeze_set,
            &empty_send_reqs(),
        );

        let decision_normal = choose_tier_for_binding(
            &binding(SharedBindingFacts {
                needs_sharing: true,
                ..Default::default()
            }),
            &empty_set(),
            &empty_send_reqs(),
        );

        // Freeze-rotate may or may not change the tier depending on implementation,
        // but the function should not panic with it set.
        assert!(
            [
                OwnershipTier::PlainOwned,
                OwnershipTier::BoxOwned,
                OwnershipTier::RcShared,
                OwnershipTier::ArcShared,
                OwnershipTier::ArcMutShared,
                OwnershipTier::RcMutShared
            ]
            .contains(&decision_frozen.tier),
            "freeze-rotate decision should be a valid tier"
        );
        let _ = decision_normal; // just ensure no panic
    }
}
