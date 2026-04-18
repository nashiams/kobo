use kobo_ir::{
    BindingUsage, FileId, KirNodeId, KoboAstNodeId, KoboSpan, SharedBindingFacts,
    TransformBindingFacts, TransformFacts, UseEvent,
};

use super::freeze_rotate::detect_freeze_and_rotate;

fn span(start: u32, end: u32) -> KoboSpan {
    KoboSpan::new(start, end, FileId(0))
}

fn make_binding(
    node_id: u32,
    name: &str,
    decl_span: KoboSpan,
    uses: Vec<UseEvent>,
) -> TransformBindingFacts {
    TransformBindingFacts {
        node: KirNodeId(node_id),
        ast_id: KoboAstNodeId(node_id),
        binding_name: name.to_owned(),
        span: decl_span,
        resource_kind: None,
        hint: None,
        hint_span: None,
        is_copy_known: false,
        is_generic: false,
        is_async: false,
        async_shared: false,
        usage: BindingUsage {
            declaration: decl_span,
            uses,
        },
        shared_facts: SharedBindingFacts {
            node_id: KirNodeId(node_id),
            needs_sharing: true,
            mutation_required: true,
            ..Default::default()
        },
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

fn make_facts(bindings: Vec<TransformBindingFacts>) -> TransformFacts {
    let usages = bindings.iter().map(|b| b.usage.clone()).collect();
    let shared = bindings.iter().map(|b| b.shared_facts.clone()).collect();
    TransformFacts {
        bindings,
        usages,
        shared_facts: shared,
        hint_conflicts: vec![],
    }
}

/// S-2 Contract Test: Move-Then-Rebind → eligible for freeze-and-rotate
/// Binding has Mutated then Moved, with NO uses after the move
#[test]
fn move_then_dead_is_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(10, 15) },
            UseEvent::Moved { span: span(20, 25), scope_depth: 0 },
            // No uses after move → eligible
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        eligible.contains(&KirNodeId(1)),
        "binding moved with no later use should be eligible"
    );
}

/// S-2 Contract Test: Used-After-Move → NOT eligible
/// Binding has Mutated, Moved, then ReadOnly → NOT eligible
#[test]
fn used_after_move_not_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(10, 15) },
            UseEvent::Moved { span: span(20, 25), scope_depth: 0 },
            UseEvent::ReadOnly { span: span(30, 35) }, // used after move!
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        !eligible.contains(&KirNodeId(1)),
        "binding used after move should NOT be eligible"
    );
}

/// S-2 Contract Test: Binding with no moves at all → NOT eligible
#[test]
fn no_move_not_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(10, 15) },
            UseEvent::ReadOnly { span: span(20, 25) },
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        !eligible.contains(&KirNodeId(1)),
        "binding without move should NOT be eligible"
    );
}

/// S-2 Contract Test: Multiple bindings, only the moved-and-dead one is eligible
#[test]
fn only_dead_moved_binding_eligible() {
    let facts = make_facts(vec![
        make_binding(
            1,
            "data",
            span(0, 5),
            vec![
                UseEvent::Mutated { span: span(10, 15) },
                UseEvent::Moved { span: span(20, 25), scope_depth: 0 },
            ],
        ),
        make_binding(
            2,
            "other",
            span(30, 35),
            vec![
                UseEvent::Mutated { span: span(40, 45) },
                UseEvent::Moved { span: span(50, 55), scope_depth: 0 },
                UseEvent::ReadOnly { span: span(60, 65) },
            ],
        ),
    ]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(eligible.contains(&KirNodeId(1)));
    assert!(!eligible.contains(&KirNodeId(2)));
}

/// S-2 Contract Test: Moved then borrowed after → NOT eligible
#[test]
fn borrowed_after_move_not_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Moved { span: span(10, 15), scope_depth: 0 },
            UseEvent::Borrowed {
                kind: kobo_ir::BorrowKind::Immutable,
                span: span(20, 25),
            },
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        !eligible.contains(&KirNodeId(1)),
        "binding borrowed after move should NOT be eligible"
    );
}

/// S-2 Integration Test: Full pipeline with real source
/// Binding moved with no subsequent usage → PlainOwned
#[test]
fn freeze_rotate_in_real_kir() {
    use kobo_ir::{FileId, NodeIdGen, OwnershipTier};
    use kobo_parser::parse_file;
    use crate::options::TransformOptions;
    use crate::transform::build_kir;

    let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    let snapshot = data;
    snapshot.len();
}
"#;

    let mut id_gen = NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());

    // data is moved to snapshot and never used after → should qualify for freeze-rotate
    let data_binding = kir
        .transform_facts()
        .iter_bindings()
        .find(|b| b.binding_name == "data")
        .expect("data binding should exist");

    let decision = kir.tier_decision(data_binding.node)
        .expect("tier decision should exist");

    // data should be PlainOwned because it's moved and dead afterward
    assert_eq!(
        decision.tier,
        OwnershipTier::PlainOwned,
        "freeze-and-rotate binding should be PlainOwned, got {:?}",
        decision.tier
    );
}

/// S-2 Contract Acceptance Test: MoveRebind reason when S-1 doesn't apply.
/// A binding with needs_sharing=true but moved-and-dead → PlainOwned(MoveRebind)
#[test]
fn move_rebind_reason_when_sharing_needed() {
    use kobo_ir::{OwnershipTier, TierReason};
    use crate::tiered::choose_tiers;

    // Build a binding where needs_sharing=true, so S-1 won't catch it,
    // but the binding is moved and dead → S-2 should catch it.
    let binding = TransformBindingFacts {
        node: KirNodeId(1),
        ast_id: KoboAstNodeId(1),
        binding_name: "data".to_owned(),
        span: span(0, 5),
        resource_kind: None,
        hint: None,
        hint_span: None,
        is_copy_known: false,
        is_generic: false,
        is_async: false,
        async_shared: false,
        usage: BindingUsage {
            declaration: span(0, 5),
            uses: vec![
                UseEvent::ReadOnly { span: span(10, 15) },
                UseEvent::Moved { span: span(20, 25), scope_depth: 0 },
                // No uses after move
            ],
        },
        shared_facts: SharedBindingFacts {
            node_id: KirNodeId(1),
            needs_sharing: true,  // S-1 won't catch this
            has_escape: false,
            mutation_required: false,
            ..Default::default()
        },
        clone_elision: None,
        elision_fallback: None,
        plain_clone_alias: false,
        plain_clone_source: None,
        plain_clone_move_span: None,
        elision_skip_reason: None,
        decl_scope_depth: 0,
        ref_returning_read_spans: Vec::new(),
    };

    let facts = make_facts(vec![binding]);
    let mut kir = kobo_ir::Kir::from_nodes(vec![]);
    kir.set_transform_facts(facts.clone());

    let decisions = choose_tiers(&facts, &mut kir);

    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].tier, OwnershipTier::PlainOwned);
    assert_eq!(decisions[0].reason, TierReason::MoveRebind);
}

/// S-2 Contract Acceptance Test: Used-after-move binding does NOT get MoveRebind
#[test]
fn no_move_rebind_when_used_after_move() {
    use kobo_ir::{OwnershipTier, TierReason};
    use crate::tiered::choose_tiers;

    let binding = TransformBindingFacts {
        node: KirNodeId(1),
        ast_id: KoboAstNodeId(1),
        binding_name: "data".to_owned(),
        span: span(0, 5),
        resource_kind: None,
        hint: None,
        hint_span: None,
        is_copy_known: false,
        is_generic: false,
        is_async: false,
        async_shared: false,
        usage: BindingUsage {
            declaration: span(0, 5),
            uses: vec![
                UseEvent::ReadOnly { span: span(10, 15) },
                UseEvent::Moved { span: span(20, 25), scope_depth: 0 },
                UseEvent::ReadOnly { span: span(30, 35) }, // used after move!
            ],
        },
        shared_facts: SharedBindingFacts {
            node_id: KirNodeId(1),
            needs_sharing: true,
            has_escape: false,
            mutation_required: false,
            ..Default::default()
        },
        clone_elision: None,
        elision_fallback: None,
        plain_clone_alias: false,
        plain_clone_source: None,
        plain_clone_move_span: None,
        elision_skip_reason: None,
        decl_scope_depth: 0,
        ref_returning_read_spans: Vec::new(),
    };

    let facts = make_facts(vec![binding]);
    let mut kir = kobo_ir::Kir::from_nodes(vec![]);
    kir.set_transform_facts(facts.clone());

    let decisions = choose_tiers(&facts, &mut kir);

    assert_eq!(decisions.len(), 1);
    // Should NOT be MoveRebind — should escalate to Rc or similar
    assert_ne!(decisions[0].reason, TierReason::MoveRebind);
}

/// BUG-13 Contract Test: Move in deeper scope (conditional) is NOT eligible.
/// A move inside an if-branch (scope_depth > decl_scope_depth) means the binding
/// may still be live in the else branch.
#[test]
fn move_in_deeper_scope_not_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(10, 15) },
            // Move at scope_depth 1 (inside if-branch), but decl is at depth 0
            UseEvent::Moved { span: span(20, 25), scope_depth: 1 },
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        !eligible.contains(&KirNodeId(1)),
        "move at deeper scope than declaration should NOT be eligible (conditional move)"
    );
}

/// BUG-14 Contract Test: Move inside a loop is NOT eligible.
/// Even if the move appears terminal in the loop body, the loop may iterate again.
#[test]
fn move_in_loop_scope_not_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(10, 15) },
            // Move at scope_depth 2 (nested loop), but decl is at depth 0
            UseEvent::Moved { span: span(20, 25), scope_depth: 2 },
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        !eligible.contains(&KirNodeId(1)),
        "move inside loop (deeper scope) should NOT be eligible"
    );
}

/// BUG-14 Contract Test: Move at same scope as declaration IS eligible.
/// This is the normal case — a move at the same level as the declaration
/// is unconditional.
#[test]
fn move_at_declaration_scope_is_eligible() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(10, 15) },
            // Move at scope_depth 0, same as decl_scope_depth 0
            UseEvent::Moved { span: span(20, 25), scope_depth: 0 },
        ],
    )]);

    let eligible = detect_freeze_and_rotate(&facts);

    assert!(
        eligible.contains(&KirNodeId(1)),
        "move at same scope as declaration SHOULD be eligible"
    );
}
