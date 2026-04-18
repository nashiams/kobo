use kobo_ir::{
    BindingUsage, FileId, KirNodeId, KoboAstNodeId, KoboSpan, SharedBindingFacts,
    TransformBindingFacts, TransformFacts, UseEvent,
};

use super::extract_borrow::find_extract_before_borrow;

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
        shared_facts: SharedBindingFacts::default(),
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

/// S-17 Contract Test: Auto-Extraction
/// Input: data has ReadOnly at span(10,15) then Mutated at span(20,30)
/// → extraction site detected with temp name __kobo_extract_0
#[test]
fn extract_detected_for_read_then_mutate() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::ReadOnly { span: span(10, 15) },
            UseEvent::Mutated { span: span(20, 30) },
        ],
    )]);

    let sites = find_extract_before_borrow(&facts);

    assert_eq!(sites.len(), 1, "should detect one extraction site");
    assert_eq!(sites[0].binding_name, "data");
    assert_eq!(sites[0].borrow_expr_span, span(10, 15));
    assert_eq!(sites[0].conflict_span, span(20, 30));
    assert_eq!(sites[0].temp_name, "__kobo_extract_0");
}

/// S-17 Contract Test: No Extraction When No Conflict
/// Input: data has ReadOnly at span(10,15), no mutation after
/// → no extraction sites
#[test]
fn no_extraction_when_no_conflict() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![UseEvent::ReadOnly { span: span(10, 15) }],
    )]);

    let sites = find_extract_before_borrow(&facts);

    assert!(
        sites.is_empty(),
        "should not extract when there is no conflicting mutation"
    );
}

/// S-17 Contract Test: No extraction when mutation comes BEFORE read
/// (no conflict: the mutation is already done by the time the read happens)
#[test]
fn no_extraction_when_mutation_before_read() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::Mutated { span: span(5, 10) },
            UseEvent::ReadOnly { span: span(15, 20) },
        ],
    )]);

    let sites = find_extract_before_borrow(&facts);

    assert!(
        sites.is_empty(),
        "should not extract when mutation is before the read"
    );
}

/// S-17 Contract Test: Multiple extractions get unique temp names
#[test]
fn multiple_extractions_get_unique_names() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::ReadOnly { span: span(10, 15) },
            UseEvent::ReadOnly { span: span(16, 19) },
            UseEvent::Mutated { span: span(20, 30) },
        ],
    )]);

    let sites = find_extract_before_borrow(&facts);

    assert_eq!(sites.len(), 2, "should detect two extraction sites");
    assert_eq!(sites[0].temp_name, "__kobo_extract_0");
    assert_eq!(sites[1].temp_name, "__kobo_extract_1");
}

/// S-17 Contract Test: Only reads that return owned values are eligible
/// Sequential read-only (no mutation) → no extraction needed
#[test]
fn no_extraction_for_sequential_reads_only() {
    let facts = make_facts(vec![make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::ReadOnly { span: span(10, 15) },
            UseEvent::ReadOnly { span: span(20, 25) },
        ],
    )]);

    let sites = find_extract_before_borrow(&facts);

    assert!(
        sites.is_empty(),
        "should not extract when there are only reads"
    );
}

/// S-17 Integration Test: Full pipeline with real source code
/// Input: fn main() { let data = vec![1,2,3]; data.len(); data.push(42); }
#[test]
fn extract_detected_in_real_kir() {
    use kobo_ir::{FileId, NodeIdGen};
    use kobo_parser::parse_file;
    use crate::options::TransformOptions;
    use crate::transform::build_kir;

    let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    data.len();
    data.push(42);
}
"#;

    let mut id_gen = NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
    let facts = kir.transform_facts();

    let sites = find_extract_before_borrow(facts);

    assert!(
        !sites.is_empty(),
        "should detect extraction opportunity for data.len() before data.push()"
    );
    assert_eq!(sites[0].binding_name, "data");
    assert!(sites[0].temp_name.starts_with("__kobo_extract_"));
}

/// S-17 Integration Test: No extraction when no conflict exists
#[test]
fn no_extraction_in_real_kir_without_conflict() {
    use kobo_ir::{FileId, NodeIdGen};
    use kobo_parser::parse_file;
    use crate::options::TransformOptions;
    use crate::transform::build_kir;

    let source = r#"
fn main() {
    let data = vec![1, 2, 3];
    let len = data.len();
    println!("{}", len);
}
"#;

    let mut id_gen = NodeIdGen::new();
    let ast = parse_file(source, FileId(0), &mut id_gen).expect("parse should succeed");
    let kir = build_kir(&ast, &mut id_gen, TransformOptions::default());
    let facts = kir.transform_facts();

    let sites = find_extract_before_borrow(facts);

    assert!(
        sites.is_empty(),
        "should not detect extraction when there is no borrow conflict"
    );
}

/// BUG-12 Contract Test: Reference-returning read before mutation does NOT trigger extraction.
/// `.iter()` returns a reference, so extracting it would move a borrow — unsound.
#[test]
fn ref_returning_read_does_not_trigger_extraction() {
    let read_span = span(10, 15);
    let mut binding = make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::ReadOnly { span: read_span },
            UseEvent::Mutated { span: span(20, 30) },
        ],
    );
    // Mark the read as a ref-returning method call (e.g., .iter())
    binding.ref_returning_read_spans.push(read_span);

    let facts = make_facts(vec![binding]);
    let sites = find_extract_before_borrow(&facts);

    assert!(
        sites.is_empty(),
        "ref-returning read (e.g. .iter()) before mutation should NOT trigger extraction"
    );
}

/// BUG-12 Contract Test: Owned-value read before mutation DOES trigger extraction.
/// `.clone()` returns an owned value, so extracting it is safe.
#[test]
fn owned_value_read_does_trigger_extraction() {
    let read_span = span(10, 15);
    let binding = make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::ReadOnly { span: read_span },
            UseEvent::Mutated { span: span(20, 30) },
        ],
    );
    // ref_returning_read_spans is empty → this is an owned-value read

    let facts = make_facts(vec![binding]);
    let sites = find_extract_before_borrow(&facts);

    assert_eq!(
        sites.len(),
        1,
        "owned-value read (e.g. .clone()) before mutation SHOULD trigger extraction"
    );
}

/// BUG-12 Contract Test: Mixed reads — only non-ref reads are eligible
#[test]
fn mixed_ref_and_owned_reads_only_owned_triggers() {
    let ref_read = span(10, 15);
    let owned_read = span(16, 19);
    let mut binding = make_binding(
        1,
        "data",
        span(0, 5),
        vec![
            UseEvent::ReadOnly { span: ref_read },
            UseEvent::ReadOnly { span: owned_read },
            UseEvent::Mutated { span: span(20, 30) },
        ],
    );
    // Only the first read is ref-returning
    binding.ref_returning_read_spans.push(ref_read);

    let facts = make_facts(vec![binding]);
    let sites = find_extract_before_borrow(&facts);

    assert_eq!(
        sites.len(),
        1,
        "only the owned-value read should trigger extraction"
    );
    assert_eq!(sites[0].borrow_expr_span, owned_read);
}
