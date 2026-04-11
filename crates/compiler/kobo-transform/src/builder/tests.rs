use kobo_ir::{
    BindingUsage, BorrowKind, EscapeKind, FileId, HintConflictFact, HintConflictReason, KoboSpan,
    OwnershipHint, OwnershipTier, TransformFacts, UseEvent,
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
