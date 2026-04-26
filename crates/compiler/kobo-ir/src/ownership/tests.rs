use super::*;
use crate::{BorrowKind, FileId, KirNodeId, KoboSpan};

fn span(start: u32) -> KoboSpan {
    KoboSpan::new(start, start + 1, FileId(0))
}

fn usage(events: Vec<UseEvent>) -> BindingUsage {
    BindingUsage {
        declaration: span(0),
        uses: events,
    }
}

#[test]
fn single_read_only_use_stays_local() {
    let shared_facts =
        derive_shared_facts(&usage(vec![UseEvent::ReadOnly { span: span(10) }]), None);

    assert!(!shared_facts.needs_sharing);
    assert!(!shared_facts.sequential_read_only);
}

#[test]
fn repeated_ephemeral_borrows_are_marked_as_sequential_read_only() {
    let shared_facts = derive_shared_facts(
        &usage(vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10),
            },
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(20),
            },
        ]),
        None,
    );

    assert!(shared_facts.needs_sharing);
    assert!(shared_facts.sequential_read_only);
}

#[test]
fn two_concurrent_borrows_require_sharing() {
    let shared_facts = derive_shared_facts(
        &usage(vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10),
            },
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(20),
            },
        ]),
        None,
    );

    assert!(shared_facts.needs_sharing);
}

#[test]
fn dead_borrow_before_move_does_not_force_sharing() {
    let shared_facts = derive_shared_facts(
        &usage(vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10),
            },
            UseEvent::Moved {
                span: span(20),
                scope_depth: 0,
            },
        ]),
        Some(&CloneElisionDecision::Move),
    );

    assert!(!shared_facts.needs_sharing);
    assert!(!shared_facts.live_borrow_at_move);
}

#[test]
fn live_borrow_at_move_requires_sharing() {
    let shared_facts = derive_shared_facts(
        &usage(vec![
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                span: span(10),
            },
            UseEvent::Moved {
                span: span(20),
                scope_depth: 0,
            },
            UseEvent::ReadOnly { span: span(30) },
        ]),
        None,
    );

    assert!(shared_facts.needs_sharing);
    assert!(shared_facts.live_borrow_at_move);
}

#[test]
fn mutated_event_sets_mutation_required() {
    let shared_facts =
        derive_shared_facts(&usage(vec![UseEvent::Mutated { span: span(10) }]), None);

    assert!(shared_facts.mutation_required);
    assert!(shared_facts.needs_mutable_wrapper);
}

#[test]
fn mutable_borrow_sets_mutation_required() {
    let shared_facts = derive_shared_facts(
        &usage(vec![UseEvent::Borrowed {
            kind: BorrowKind::Mutable,
            span: span(10),
        }]),
        None,
    );

    assert!(shared_facts.mutation_required);
    assert!(shared_facts.needs_mutable_wrapper);
}

#[test]
fn shared_binding_facts_are_a_pure_function_of_usage() {
    let usage = usage(vec![
        UseEvent::Borrowed {
            kind: BorrowKind::Immutable,
            span: span(10),
        },
        UseEvent::Moved {
            span: span(20),
            scope_depth: 0,
        },
        UseEvent::ReadOnly { span: span(30) },
    ]);

    let first = derive_shared_facts(&usage, None);
    let second = derive_shared_facts(&usage, None);

    assert_eq!(first, second);
}

#[test]
fn derive_transform_facts_populates_schema_for_all_usage_kinds() {
    let facts = derive_transform_facts(
        vec![usage(vec![
            UseEvent::ReadOnly { span: span(10) },
            UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                span: span(20),
            },
            UseEvent::Escaped {
                kind: EscapeKind::StoredInStruct,
                span: span(30),
            },
            UseEvent::Moved {
                span: span(40),
                scope_depth: 0,
            },
        ])],
        vec![CloneElisionDecision::Clone],
        vec![HintConflictFact {
            node: KirNodeId(1),
            hint: OwnershipHint::Shared,
            hint_span: span(50),
            conflict_span: span(60),
            chosen_tier: OwnershipTier::RcMutShared,
            reason: HintConflictReason::MutableUse,
        }],
    );

    assert_eq!(facts.usages.len(), 1);
    assert_eq!(facts.shared_facts.len(), 1);
    assert_eq!(facts.hint_conflicts.len(), 1);

    let shared_facts = &facts.shared_facts[0];
    assert!(shared_facts.mutation_required);
    assert_eq!(shared_facts.escape_floor, Some(EscapeKind::StoredInStruct));
    assert!(!shared_facts.borrow_sites.is_empty());
}

#[test]
fn priority_orders_floor_strength_not_candidate_ladder() {
    assert!(OwnershipTier::PlainOwned.priority() < OwnershipTier::RcShared.priority());
    assert!(OwnershipTier::RcShared.priority() < OwnershipTier::ArcShared.priority());
    assert!(OwnershipTier::ArcShared.priority() < OwnershipTier::BoxOwned.priority());
    assert!(OwnershipTier::BoxOwned.priority() < OwnershipTier::RcMutShared.priority());
}

#[test]
fn greedy_priority_orders_the_user_facing_ladder() {
    assert!(
        OwnershipTier::PlainOwned.greedy_priority() < OwnershipTier::BoxOwned.greedy_priority()
    );
    assert!(OwnershipTier::BoxOwned.greedy_priority() < OwnershipTier::RcShared.greedy_priority());
    assert!(OwnershipTier::RcShared.greedy_priority() < OwnershipTier::ArcShared.greedy_priority());
    assert!(
        OwnershipTier::ArcShared.greedy_priority() < OwnershipTier::RcMutShared.greedy_priority()
    );
}

// ── BUG-10: ArcMutShared label must say rwlock, not mutex ──

#[test]
fn arc_mut_shared_label_says_rwlock_not_mutex() {
    let label = OwnershipTier::ArcMutShared.label();
    assert_eq!(
        label, "arc_rwlock",
        "BUG-10: ArcMutShared label must be arc_rwlock, got {label}"
    );
    assert!(
        !label.contains("mutex"),
        "BUG-10: label must not mention mutex"
    );
}
