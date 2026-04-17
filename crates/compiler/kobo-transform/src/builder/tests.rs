use kobo_ir::{
    BindingUsage, BorrowKind, EscapeKind, FileId, HintConflictFact, HintConflictReason, KoboSpan,
    OwnershipHint, OwnershipTier, TierDecision, TierReason, TransformFacts, UseEvent,
};
use kobo_parser::parse_file;

use crate::finalize::finalize_transform;
use crate::options::TransformOptions;
use crate::transform::build_kir;

use super::build_transform_builder;

type Facts = TransformFacts;

fn span(start: u32) -> KoboSpan {
    KoboSpan::new(start, start + 1, FileId(0))
}

fn raw_builder_output_for(source: &str) -> super::BuilderOutput {
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    build_transform_builder(&ast, &mut id_gen, TransformOptions::default()).finish()
}

fn raw_facts_for(source: &str) -> Facts {
    raw_builder_output_for(source).transform_facts
}

fn transform_facts_for(source: &str) -> Facts {
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    finalize_transform(build_transform_builder(
        &ast,
        &mut id_gen,
        TransformOptions::default(),
    ))
    .transform_facts
}

fn raw_usage_for(source: &str, name: &str, occurrence: usize) -> BindingUsage {
    raw_facts_for(source)
        .iter_bindings()
        .filter(|binding| binding.binding_name == name)
        .nth(occurrence)
        .expect("binding should exist")
        .usage
        .clone()
}

fn raw_binding_for(source: &str, name: &str, occurrence: usize) -> kobo_ir::TransformBindingFacts {
    raw_facts_for(source)
        .iter_bindings()
        .filter(|binding| binding.binding_name == name)
        .nth(occurrence)
        .expect("binding should exist")
        .clone()
}

fn finalized_binding_for(
    source: &str,
    name: &str,
    occurrence: usize,
) -> kobo_ir::TransformBindingFacts {
    transform_facts_for(source)
        .iter_bindings()
        .filter(|binding| binding.binding_name == name)
        .nth(occurrence)
        .expect("binding should exist")
        .clone()
}

fn tier_for_binding(source: &str, name: &str, occurrence: usize) -> OwnershipTier {
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
    let binding = kir
        .transform_facts()
        .iter_bindings()
        .filter(|binding| binding.binding_name == name)
        .nth(occurrence)
        .expect("binding should exist");
    kir.tier_decision(binding.node)
        .expect("decision should exist")
        .tier
}

#[test]
fn builder_finish_sorts_recorded_use_events() {
    let source = r#"
fn main() {
    let x = String::from("hello");
}
"#;

    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let mut builder = build_transform_builder(&ast, &mut id_gen, TransformOptions::default());
    let binding_node = builder
        .transform_facts
        .iter_bindings()
        .find(|binding| binding.binding_name == "x")
        .expect("x binding should exist")
        .node;
    builder.record_event(binding_node, UseEvent::ReadOnly { span: span(20) });
    builder.record_event(binding_node, UseEvent::ReadOnly { span: span(10) });

    let output = builder.finish();
    let binding = output
        .transform_facts
        .iter_bindings()
        .find(|binding| binding.binding_name == "x")
        .expect("x binding should exist");
    let starts = binding
        .usage
        .uses
        .iter()
        .map(|event| event.span().start)
        .collect::<Vec<_>>();

    assert_eq!(starts, vec![10, 20]);
}

#[test]
fn builder_finish_preserves_recorded_hint_conflicts() {
    let source = r#"
fn main() {
    let x = String::from("hello");
}
"#;

    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let mut builder = build_transform_builder(&ast, &mut id_gen, TransformOptions::default());
    let binding_node = builder
        .transform_facts
        .iter_bindings()
        .find(|binding| binding.binding_name == "x")
        .expect("x binding should exist")
        .node;
    let conflict = HintConflictFact {
        node: binding_node,
        hint: OwnershipHint::Shared,
        hint_span: span(10),
        conflict_span: span(20),
        chosen_tier: OwnershipTier::RcShared,
        reason: HintConflictReason::SharedUsage,
    };

    builder.record_hint_conflict(conflict.clone());

    let output = builder.finish();

    assert_eq!(output.transform_facts.hint_conflicts, vec![conflict]);
}

#[test]
fn escape_analysis_is_deterministic_for_same_source() {
    let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    let z = x;
    use_ref(y);
    println!("{}", z.len());
}
"#;

    let first = raw_facts_for(source);
    let second = raw_facts_for(source);

    assert_eq!(first.usages, second.usages);
}

#[test]
fn shadowed_bindings_stay_separate_in_builder_output() {
    let source = r#"
fn main() {
    let x = String::from("outer");
    {
        let mut x: Vec<String> = Vec::new();
        x.push(String::from("inner"));
        println!("{}", x.len());
    }
    println!("{}", x.len());
}
"#;

    let facts = raw_facts_for(source);
    let xs = facts
        .iter_bindings()
        .filter(|binding| binding.binding_name == "x")
        .collect::<Vec<_>>();

    assert_eq!(xs.len(), 2);
    assert_ne!(xs[0].node, xs[1].node);
    assert_ne!(xs[0].usage.declaration, xs[1].usage.declaration);
}

#[test]
fn shadowed_binding_does_not_confuse_clone_elision_liveness() {
    let source = r#"
fn main() {
    let x = String::from("outer");
    let y = x;
    {
        let x = String::from("inner");
        println!("{}", x.len());
    }
    println!("{}", y.len());
}
"#;

    let outer = finalized_binding_for(source, "x", 0);
    let inner = finalized_binding_for(source, "x", 1);

    assert_eq!(
        outer.clone_elision,
        Some(kobo_ir::CloneElisionDecision::Move)
    );
    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::PlainOwned);
    assert_eq!(inner.clone_elision, None);
}

#[test]
fn local_only_heap_value_has_zero_escape_events() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    println!("done");
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage
        .uses
        .iter()
        .all(|event| !matches!(event, UseEvent::Escaped { .. })));
}

#[test]
fn shared_borrow_is_not_recorded_as_escape() {
    let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    use_ref(y);
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage.uses.iter().any(|event| {
        matches!(
            event,
            UseEvent::Borrowed {
                kind: BorrowKind::Immutable,
                ..
            }
        )
    }));
    assert!(usage
        .uses
        .iter()
        .all(|event| !matches!(event, UseEvent::Escaped { .. })));
}

#[test]
fn mutable_borrow_is_not_recorded_as_escape() {
    let source = r#"
fn use_mut_ref(value: &mut String) {
    value.push('!');
}

fn main() {
    let mut x = String::from("hello");
    let y = &mut x;
    use_mut_ref(y);
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage.uses.iter().any(|event| {
        matches!(
            event,
            UseEvent::Borrowed {
                kind: BorrowKind::Mutable,
                ..
            }
        )
    }));
    assert!(usage
        .uses
        .iter()
        .all(|event| !matches!(event, UseEvent::Escaped { .. })));
}

#[test]
fn return_records_return_escape_on_binding() {
    let source = r#"
fn make_name() -> String {
    let x = String::from("hello");
    return x;
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage.uses.iter().any(|event| {
        matches!(
            event,
            UseEvent::Escaped {
                kind: EscapeKind::ReturnedFromFunction,
                ..
            }
        )
    }));
}

#[test]
fn returning_parameter_does_not_record_return_escape_on_binding() {
    let source = r#"
fn id<T>(x: T) -> T {
    return x;
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage
        .uses
        .iter()
        .all(|event| !matches!(event, UseEvent::Escaped { .. })));
}

#[test]
fn field_assignment_records_struct_escape_on_binding() {
    let source = r#"
struct Store {
    field: String,
}

fn main() {
    let mut store = Store {
        field: String::from("seed"),
    };
    let x = String::from("hello");
    store.field = x;
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage.uses.iter().any(|event| {
        matches!(
            event,
            UseEvent::Escaped {
                kind: EscapeKind::StoredInStruct,
                ..
            }
        )
    }));
}

#[test]
fn opaque_call_records_opaque_escape_on_binding() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    opaque_sink(x);
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage.uses.iter().any(|event| {
        matches!(
            event,
            UseEvent::Escaped {
                kind: EscapeKind::PassedToOpaqueCall,
                ..
            }
        )
    }));
}

#[test]
fn local_function_call_does_not_record_opaque_escape_on_binding() {
    let source = r#"
fn sink(_value: String) {}

fn main() {
    let x = String::from("hello");
    sink(x);
}
"#;

    let usage = raw_usage_for(source, "x", 0);

    assert!(usage
        .uses
        .iter()
        .all(|event| !matches!(event, UseEvent::Escaped { .. })));
}

#[test]
fn two_read_only_method_calls_record_two_read_events() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    x.len();
    x.len();
}
"#;

    let usage = raw_usage_for(source, "x", 0);
    let read_count = usage
        .uses
        .iter()
        .filter(|event| matches!(event, UseEvent::ReadOnly { .. }))
        .count();

    assert_eq!(read_count, 2);
    assert_eq!(usage.uses.len(), 2);
}

#[test]
fn two_mutating_methods_record_two_mutated_events() {
    let source = r#"
fn main() {
    let mut x: Vec<String> = Vec::new();
    x.push(String::from("a"));
    x.push(String::from("b"));
}
"#;

    let usage = raw_usage_for(source, "x", 0);
    let mutate_count = usage
        .uses
        .iter()
        .filter(|event| matches!(event, UseEvent::Mutated { .. }))
        .count();

    assert_eq!(mutate_count, 2);
    assert_eq!(usage.uses.len(), 2);
}

#[test]
fn conditional_mutation_marks_mutable_usage_inside_if_branch() {
    let source = r#"
fn mutate<T>(_value: &mut T) {}

fn main() {
    let x = String::from("kobo");
    if true {
        mutate(&mut x);
    }
    println!("{}", x.len());
}
"#;

    let binding = finalized_binding_for(source, "x", 0);

    assert!(binding.usage.uses.iter().any(|event| {
        matches!(
            event,
            UseEvent::Mutated { .. }
                | UseEvent::Borrowed {
                    kind: kobo_ir::BorrowKind::Mutable,
                    ..
                }
        )
    }));
    assert!(binding.shared_facts.needs_mutable_wrapper);
}

#[test]
fn match_branch_mutation_marks_mutable_usage() {
    let source = r#"
fn main() {
    let mut x: Vec<String> = Vec::new();
    let value = 1;
    match value {
        0 => x.push(String::from("a")),
        _ => (),
    }
}
"#;

    let binding = finalized_binding_for(source, "x", 0);

    assert!(binding
        .usage
        .uses
        .iter()
        .any(|event| matches!(event, UseEvent::Mutated { .. })));
    assert!(binding.shared_facts.needs_mutable_wrapper);
}

#[test]
fn else_if_branch_mutation_marks_mutable_usage() {
    let source = r#"
fn main() {
    let mut x: Vec<String> = Vec::new();
    if false {
        ()
    } else if true {
        x.push(String::from("a"));
    }
}
"#;

    let binding = finalized_binding_for(source, "x", 0);

    assert!(binding
        .usage
        .uses
        .iter()
        .any(|event| matches!(event, UseEvent::Mutated { .. })));
    assert!(binding.shared_facts.needs_mutable_wrapper);
}

#[test]
fn read_only_if_branch_does_not_require_mutable_wrapper() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    if true {
        x.len();
    }
}
"#;

    let binding = finalized_binding_for(source, "x", 0);

    assert!(binding
        .usage
        .uses
        .iter()
        .all(|event| matches!(event, UseEvent::ReadOnly { .. })));
    assert!(!binding.shared_facts.needs_mutable_wrapper);
}

#[test]
fn loop_bodies_are_visited_for_mutable_usage() {
    let sources = [
        r#"
fn main() {
    let mut x: Vec<String> = Vec::new();
    while true {
        x.push(String::from("a"));
        break;
    }
}
"#,
        r#"
fn main() {
    let mut x: Vec<String> = Vec::new();
    for value in 0..1 {
        let _ = value;
        x.push(String::from("a"));
    }
}
"#,
        r#"
fn main() {
    let mut x: Vec<String> = Vec::new();
    loop {
        x.push(String::from("a"));
        break;
    }
}
"#,
    ];

    for source in sources {
        let binding = finalized_binding_for(source, "x", 0);
        assert!(binding
            .usage
            .uses
            .iter()
            .any(|event| matches!(event, UseEvent::Mutated { .. })));
        assert!(binding.shared_facts.needs_mutable_wrapper);
    }
}

#[test]
fn live_borrow_at_move_sets_shared_fact() {
    let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    let z = x;
    use_ref(y);
    println!("{}", z.len());
}
"#;

    let facts = transform_facts_for(source);
    let binding = facts
        .iter_bindings()
        .find(|binding| binding.binding_name == "x")
        .expect("x binding should exist");

    assert!(binding.shared_facts.live_borrow_at_move);
    assert!(binding.shared_facts.needs_sharing);
}

#[test]
fn consumed_borrow_before_move_stays_plain_owned() {
    let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    use_ref(y);
    let z = x;
    println!("{}", z.len());
}
"#;

    let binding = finalized_binding_for(source, "x", 0);

    assert!(!binding.shared_facts.live_borrow_at_move);
    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::PlainOwned);
}

#[test]
fn dead_borrow_move_excludes_dead_alias_reads_from_final_usage() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    let y = &x;
    let z = x;
    println!("{}", z.len());
}
"#;

    let binding = finalized_binding_for(source, "x", 0);
    let read_events = binding
        .usage
        .uses
        .iter()
        .filter(|event| matches!(event, UseEvent::ReadOnly { .. }))
        .count();
    let borrow_events = binding
        .usage
        .uses
        .iter()
        .filter(|event| {
            matches!(
                event,
                UseEvent::Borrowed {
                    kind: BorrowKind::Immutable,
                    ..
                }
            )
        })
        .count();

    assert_eq!(
        binding.clone_elision,
        Some(kobo_ir::CloneElisionDecision::Move)
    );
    assert_eq!(read_events, 0);
    assert_eq!(borrow_events, 1);
    assert!(!binding.shared_facts.needs_sharing);
    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::PlainOwned);
}

#[test]
fn builder_records_clone_elision_candidates_for_dead_original_alias_sites() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    let y = x;
    println!("{}", y.len());
}
"#;

    let output = raw_builder_output_for(source);

    assert_eq!(output.clone_elision_candidates.len(), 1);
}

#[test]
fn noncandidate_move_alias_counts_assignment_site_toward_sharing() {
    let source = r#"
fn main() {
    let x = String::from("hello");
    let y = x;
    x.len();
    let _ = y;
}
"#;

    let binding = finalized_binding_for(source, "x", 0);

    assert!(binding.shared_facts.needs_sharing);
    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::RcShared);
}

#[test]
fn small_clone_eligible_alias_marks_plain_clone_site() {
    let source = r#"
#[derive(Clone, Copy)]
struct SmallCopy {
    a: u64,
    b: u64,
}

fn main() {
    let x = SmallCopy { a: 1, b: 2 };
    let y = x;
    let _sum = x.a + y.b;
}
"#;

    let alias_binding = raw_binding_for(source, "y", 0);
    let source_binding = finalized_binding_for(source, "x", 0);

    assert!(alias_binding.plain_clone_alias);
    assert_eq!(source_binding.clone_elision, None);
    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::PlainOwned);
}

#[test]
fn opaque_small_clone_site_records_skip_reason_for_annotation() {
    let source = r#"
use std::path::PathBuf;

#[derive(Clone)]
struct MaybeAlloc {
    path: PathBuf,
}

fn main() {
    let x = MaybeAlloc {
        path: PathBuf::from("Cargo.toml"),
    };
    let y = x;
    let _ = x.path.display();
    let _ = y.path.display();
}
"#;

    let alias_binding = raw_binding_for(source, "y", 0);

    assert_eq!(
        alias_binding.elision_skip_reason,
        Some(kobo_ir::ElisionSkipReason::FieldTypeUnknownMayAllocate)
    );
    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::RcShared);
}

#[test]
fn local_borrow_only_stays_plain_owned() {
    let source = r#"
fn use_ref(value: &String) {
    println!("{}", value.len());
}

fn main() {
    let x = String::from("hello");
    let y = &x;
    use_ref(y);
}
"#;

    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::PlainOwned);
}

#[test]
fn read_then_mutate_escalates_to_rc_refcell() {
    let source = r#"
fn use_read(value: &String) {
    println!("{}", value.len());
}

fn use_mut(value: &mut String) {
    value.push('!');
}

fn main() {
    let mut x = String::from("hello");
    use_read(&x);
    use_mut(&mut x);
}
"#;

    assert_eq!(tier_for_binding(source, "x", 0), OwnershipTier::RcMutShared);
}

#[test]
fn structurally_identical_generics_choose_same_tier() {
    let source = r#"
fn use_mut<T>(_value: &mut T) {}

fn first<T>(mut value: T) {
    use_mut(&mut value);
}

fn second<U>(mut value: U) {
    use_mut(&mut value);
}
"#;

    assert_eq!(
        tier_for_binding(source, "value", 0),
        tier_for_binding(source, "value", 1),
    );
}

// ---------------------------------------------------------------------------
// G5: #[kobo::relax] attribute parsing tests
// ---------------------------------------------------------------------------

#[test]
fn test_relax_attr_parsing_valid_bare_records_fn_range() {
    // Valid #[kobo::relax] on a function → one entry in relaxed_fn_ranges.
    let source = r#"
#[kobo::relax]
fn relaxed() {
    let x = String::from("hello");
}
"#;
    let output = raw_builder_output_for(source);
    assert_eq!(output.relaxed_fn_ranges.len(), 1,
        "one relaxed fn range expected");
    assert!(output.relax_attr_errors.is_empty(),
        "no attr errors expected for valid bare attr");
}

#[test]
fn test_relax_attr_parsing_no_attr_records_nothing() {
    // No #[kobo::relax] → relaxed_fn_ranges is empty.
    let source = r#"
fn normal() {
    let x = String::from("hello");
}
"#;
    let output = raw_builder_output_for(source);
    assert!(output.relaxed_fn_ranges.is_empty(),
        "no relaxed ranges expected when attr absent");
    assert!(output.relax_attr_errors.is_empty());
}

#[test]
fn test_relax_attr_parsing_with_value_argument_produces_warning() {
    // #[kobo::relax = "reason"] → warning "takes no arguments".
    let source = r#"
#[kobo::relax = "testing"]
fn foo() {}
"#;
    let output = raw_builder_output_for(source);
    assert!(output.relaxed_fn_ranges.is_empty(),
        "malformed attr must not record a relaxed range");
    assert_eq!(output.relax_attr_errors.len(), 1);
    assert!(!output.relax_attr_errors[0].is_error,
        "malformed attr should be a warning, not an error");
    assert!(output.relax_attr_errors[0].message.contains("takes no arguments"),
        "message should mention takes no arguments");
}

#[test]
fn test_relax_attr_parsing_with_list_argument_produces_warning() {
    // #[kobo::relax("something")] → warning "takes no arguments".
    let source = r#"
#[kobo::relax("something")]
fn foo() {}
"#;
    let output = raw_builder_output_for(source);
    assert!(output.relaxed_fn_ranges.is_empty(),
        "malformed attr must not record a relaxed range");
    assert_eq!(output.relax_attr_errors.len(), 1);
    assert!(!output.relax_attr_errors[0].is_error,
        "malformed attr should be a warning, not an error");
}

#[test]
fn test_relax_attr_parsing_on_struct_produces_error() {
    // #[kobo::relax] on a struct → error "can only be applied to functions".
    let source = r#"
#[kobo::relax]
struct Foo {
    x: i32,
}
"#;
    let output = raw_builder_output_for(source);
    assert!(output.relaxed_fn_ranges.is_empty());
    assert_eq!(output.relax_attr_errors.len(), 1);
    assert!(output.relax_attr_errors[0].is_error,
        "non-fn attachment should be an error");
    assert!(output.relax_attr_errors[0].message.contains("can only be applied to functions"));
}

#[test]
fn test_relax_attr_parsing_duplicate_on_same_fn_produces_lint_warning() {
    // Two #[kobo::relax] on the same function → second is a lint warning.
    let source = r#"
#[kobo::relax]
#[kobo::relax]
fn foo() {}
"#;
    let output = raw_builder_output_for(source);
    // First attr records the range; second is a duplicate warning.
    assert_eq!(output.relaxed_fn_ranges.len(), 1,
        "only one range recorded (first occurrence)");
    assert_eq!(output.relax_attr_errors.len(), 1,
        "one duplicate lint expected");
    assert!(!output.relax_attr_errors[0].is_error,
        "duplicate is a warning, not an error");
    assert!(output.relax_attr_errors[0].message.contains("duplicate"));
}

#[test]
fn test_relax_attr_parsing_multiple_relaxed_fns_in_same_file() {
    // Two functions each with #[kobo::relax] → two ranges.
    let source = r#"
#[kobo::relax]
fn first() {}

#[kobo::relax]
fn second() {}
"#;
    let output = raw_builder_output_for(source);
    assert_eq!(output.relaxed_fn_ranges.len(), 2,
        "two relaxed fn ranges expected");
    assert!(output.relax_attr_errors.is_empty());
}

#[test]
fn test_relax_attr_fn_range_covers_function_body() {
    // The recorded range covers the full function span (start < end, covers body).
    let source = r#"
#[kobo::relax]
fn relaxed() {
    let _x = 42;
}
"#;
    let output = raw_builder_output_for(source);
    assert_eq!(output.relaxed_fn_ranges.len(), 1);
    let range = output.relaxed_fn_ranges[0];
    assert!(range.start < range.end, "fn span must be non-empty");
}

#[test]
fn test_relax_attr_parsing_unrelaxed_fn_in_same_file_not_recorded() {
    // Only the relaxed fn is in relaxed_fn_ranges; plain fns are absent.
    let source = r#"
#[kobo::relax]
fn relaxed() {}

fn plain() {}
"#;
    let output = raw_builder_output_for(source);
    assert_eq!(output.relaxed_fn_ranges.len(), 1,
        "only one range — the relaxed fn");
}

// ── BUG-11: duplicate #[kobo::migrate] on parameters ──

#[test]
fn test_migrate_duplicate_on_parameter_produces_lint_warning() {
    let source = r#"
fn foo(#[kobo::migrate] #[kobo::migrate] x: String) {}
"#;
    let output = raw_builder_output_for(source);
    // Only first attr should be recorded as a migrate site.
    assert_eq!(output.migrate_sites.len(), 1,
        "only one migrate site recorded (first occurrence)");
    // Second attr produces a duplicate lint warning.
    let dup_errors: Vec<_> = output.relax_attr_errors.iter()
        .filter(|e| e.message.contains("duplicate") && e.message.contains("migrate"))
        .collect();
    assert_eq!(dup_errors.len(), 1, "one duplicate migrate lint expected on param");
    assert!(!dup_errors[0].is_error, "duplicate is a warning, not an error");
}

// ── BUG-11: duplicate #[kobo::migrate] on let-bindings ──

#[test]
fn test_migrate_duplicate_on_let_binding_produces_lint_warning() {
    let source = r#"
fn main() {
    #[kobo::migrate]
    #[kobo::migrate]
    let x = String::from("hello");
}
"#;
    let output = raw_builder_output_for(source);
    // Only first attr should be recorded as a migrate site.
    let let_sites: Vec<_> = output.migrate_sites.iter()
        .filter(|s| s.target == kobo_ir::MigrateTarget::LetBinding)
        .collect();
    assert_eq!(let_sites.len(), 1,
        "only one migrate site recorded for let-binding (first occurrence)");
    // Second attr produces a duplicate lint warning.
    let dup_errors: Vec<_> = output.relax_attr_errors.iter()
        .filter(|e| e.message.contains("duplicate") && e.message.contains("migrate"))
        .collect();
    assert_eq!(dup_errors.len(), 1, "one duplicate migrate lint expected on let-binding");
    assert!(!dup_errors[0].is_error, "duplicate is a warning, not an error");
}

#[test]
fn test_build_kir_handles_impl_block() {
    let source = r#"
struct Counter { val: i32 }
impl Counter {
    fn increment(&mut self) {
        let step = 1;
        self.val += step;
    }
    fn value(&self) -> i32 {
        self.val
    }
}
"#;
    let facts = raw_facts_for(source);
    // The `step` local inside `increment` must be discovered.
    let step_binding = facts
        .iter_bindings()
        .find(|b| b.binding_name == "step");
    assert!(step_binding.is_some(), "local `step` inside impl method must be walked");
}

#[test]
fn test_qualified_path_no_collision() {
    let source = r#"
struct Foo { data: Vec<i32> }
struct Bar { data: Vec<i32> }
impl Foo {
    fn process(&self) { let x = self.data.clone(); }
}
impl Bar {
    fn process(&self) { let x = self.data.clone(); }
}
"#;
    let facts = raw_facts_for(source);
    let x_bindings: Vec<_> = facts
        .iter_bindings()
        .filter(|b| b.binding_name == "x")
        .collect();
    assert_eq!(x_bindings.len(), 2, "two independent x bindings from different impl blocks");
}

fn tier_decision_for_binding(source: &str, name: &str, occurrence: usize) -> TierDecision {
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
    let binding = kir
        .transform_facts()
        .iter_bindings()
        .filter(|binding| binding.binding_name == name)
        .nth(occurrence)
        .expect("binding should exist");
    kir.tier_decision(binding.node)
        .expect("decision should exist")
        .clone()
}

fn tier_decision_for_binding_with_options(
    source: &str,
    name: &str,
    occurrence: usize,
    options: TransformOptions,
) -> TierDecision {
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, options);
    let binding = kir
        .transform_facts()
        .iter_bindings()
        .filter(|binding| binding.binding_name == name)
        .nth(occurrence)
        .expect("binding should exist");
    kir.tier_decision(binding.node)
        .expect("decision should exist")
        .clone()
}

#[test]
fn test_derive_copy_detected() {
    let source = r#"
#[derive(Clone, Copy)]
struct Vec3 { x: f32, y: f32, z: f32 }

fn main() {
    let pos = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
    let pos2 = pos;
    println!("{}", pos.x);
}
"#;
    let decision = tier_decision_for_binding(source, "pos", 0);
    assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    assert!(matches!(decision.reason, TierReason::CopyType));
}

#[test]
fn test_copy_type_from_config() {
    let source = r#"
use glam::Vec3;
fn main() {
    let pos: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    let pos2 = pos;
    println!("{:?}", pos);
}
"#;
    let mut options = TransformOptions::default();
    options.copy_types = vec!["glam::Vec3".to_string(), "glam::Quat".to_string()];
    let decision = tier_decision_for_binding_with_options(source, "pos", 0, options);
    assert_eq!(decision.tier, OwnershipTier::PlainOwned);
    assert!(matches!(decision.reason, TierReason::CopyType));
}

#[test]
fn test_transitive_copy() {
    let source = r#"
#[derive(Clone, Copy)]
struct Point { x: f32, y: f32 }

#[derive(Clone, Copy)]
struct Rect { top_left: Point, bottom_right: Point }

fn main() {
    let r = Rect { top_left: Point { x: 0.0, y: 0.0 }, bottom_right: Point { x: 1.0, y: 1.0 } };
    let r2 = r;
    println!("{}", r.top_left.x);
}
"#;
    let decision = tier_decision_for_binding(source, "r", 0);
    assert_eq!(decision.tier, OwnershipTier::PlainOwned);
}

#[test]
fn test_non_copy_field_blocks_copy() {
    let source = r#"
#[derive(Clone, Copy)]
struct Id(u32);

struct Named { id: Id, name: String }

fn main() {
    let n = Named { id: Id(1), name: "test".to_string() };
    let n2 = n;
    n.id;
    let _ = n2;
}
"#;
    // Named does NOT have #[derive(Copy)] → n must NOT be PlainOwned.
    // Move alias (n → n2) plus later use of n forces sharing.
    let decision = tier_decision_for_binding(source, "n", 0);
    assert_ne!(decision.tier, OwnershipTier::PlainOwned);
}

// ---------------------------------------------------------------------------
// Phase 4: Mutable method detection via MethodRegistry
// ---------------------------------------------------------------------------

#[test]
fn test_mut_self_receiver_detected() {
    // `add_item` is NOT in the hardcoded list — only MethodRegistry should detect it.
    let source = r#"
struct Stack { items: Vec<i32> }
impl Stack {
    fn add_item(&mut self, val: i32) { self.items.push(val); }
    fn peek(&self) -> Option<&i32> { self.items.last() }
}
fn main() {
    let s = Stack { items: vec![] };
    s.add_item(42);
    let _top = s.peek();
}
"#;
    let s = finalized_binding_for(source, "s", 0);
    assert!(
        s.shared_facts.mutation_required,
        "add_item(&mut self) should be detected as mutating via MethodRegistry"
    );
}

#[test]
fn test_immutable_self_receiver_not_mutating() {
    let source = r#"
struct Counter { n: u32 }
impl Counter {
    fn value(&self) -> u32 { self.n }
}
fn main() {
    let c = Counter { n: 0 };
    let _v = c.value();
}
"#;
    let c = finalized_binding_for(source, "c", 0);
    assert!(
        !c.shared_facts.mutation_required,
        "value(&self) should not be detected as mutating"
    );
}

#[test]
fn test_method_registry_overrides_hardcoded_list() {
    // `push` IS in the hardcoded list, but `add_item` is NOT.
    // Both should be detected as mutating when MethodRegistry is used.
    let source = r#"
struct Queue { items: Vec<i32> }
impl Queue {
    fn enqueue(&mut self, val: i32) { self.items.push(val); }
    fn size(&self) -> usize { self.items.len() }
}
fn main() {
    let q = Queue { items: vec![] };
    q.enqueue(10);
    let _s = q.size();
}
"#;
    let q = finalized_binding_for(source, "q", 0);
    assert!(
        q.shared_facts.mutation_required,
        "enqueue(&mut self) should be detected via MethodRegistry"
    );
}

#[test]
fn test_mutating_methods_config_override() {
    // `process` takes `&self`, but config says it's mutating.
    // Config override should mark it as mutating anyway.
    let source = r#"
struct Worker { data: Vec<i32> }
impl Worker {
    fn process(&self) -> usize { self.data.len() }
}
fn main() {
    let w = Worker { data: vec![] };
    let _n = w.process();
}
"#;
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let mut options = TransformOptions::default();
    options.mutating_methods = vec!["Worker::process".to_string()];
    let kir = build_kir(&ast, &mut id_gen, options);
    // The method_mutability map should have "process" = true from config override
    assert_eq!(
        kir.method_mutability().get("process"),
        Some(&true),
        "config override should mark process as mutating"
    );
}

#[test]
fn test_rc_elision_local_only() {
    // Local binding with no sharing, no escape → PlainOwned.
    let source = r#"
fn main() {
    let buffer = vec![0u8; 1024];
    buffer.push(42);
    println!("len = {}", buffer.len());
}
"#;
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
    // Find the tier decision for `buffer`.
    let buffer_binding = kir
        .transform_facts()
        .bindings
        .iter()
        .find(|b| b.binding_name == "buffer")
        .expect("should have buffer binding");
    let buffer_decision = kir
        .tier_decision(buffer_binding.node)
        .expect("should have a tier decision for buffer");
    assert_eq!(
        buffer_decision.tier,
        OwnershipTier::PlainOwned,
        "local-only binding should be PlainOwned, got {:?}",
        buffer_decision
    );
}

#[test]
fn test_rc_elision_shared_still_wraps() {
    // Binding passed to two different function calls → needs sharing → not PlainOwned.
    let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    let a = data.len();
    let b = data.len();
    let c = data.is_empty();
}
"#;
    let mut id_gen = kobo_ir::NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
    let data_binding = kir
        .transform_facts()
        .bindings
        .iter()
        .find(|b| b.binding_name == "data")
        .expect("should have data binding");
    let data_decision = kir
        .tier_decision(data_binding.node)
        .expect("should have a tier decision for data");
    // data has multiple read sites → needs sharing → should NOT be PlainOwned.
    assert!(
        data_decision.tier != OwnershipTier::PlainOwned,
        "shared binding should not be PlainOwned, got {:?}",
        data_decision
    );
}
