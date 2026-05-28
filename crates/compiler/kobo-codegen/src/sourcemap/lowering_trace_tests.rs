use std::collections::BTreeSet;
use std::path::Path;

use kobo_ir::{
    FileId, ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioCoreTerminatorKind,
    ScenarioCoverageFacts, ScenarioExternalCallShape, ScenarioLifecycleTemplate, ScenarioOp,
    ScenarioOpKind, ScenarioProgram,
};

use super::{build_lowering_trace, wrap_source_map, RsSpan, SourceMapEntry};

#[test]
fn lowering_trace_schema_covers_translation_event_kinds() {
    let file_id = FileId(1);
    let span = kobo_ir::KoboSpan::new(10, 20, file_id);
    let operations = trace_kind_operations(span);
    let program = ScenarioProgram {
        file_id,
        target: "trace_case".to_owned(),
        source_hash: "source".to_owned(),
        operations,
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };
    let entries = program
        .operations
        .iter()
        .enumerate()
        .map(|(order, operation)| SourceMapEntry {
            id: format!("proof-map-trace_case-{order}"),
            core_event_id: Some(super::core_event_id_for_operation(
                "trace_case",
                order,
                &operation.kind,
            )),
            binding_name: "delivery".to_owned(),
            kobo_span: span,
            rs_span: RsSpan {
                line: order + 1,
                column_start: 1,
                column_end: 8,
            },
            ownership_tier: "proof-event".to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: None,
        })
        .collect();
    let source_map = wrap_source_map(
        Path::new("src/main.kobo"),
        Path::new("src/main.rs"),
        entries,
    );
    let kinds = build_lowering_trace(&[program], &source_map)
        .into_iter()
        .map(|event| event.kind)
        .collect::<BTreeSet<_>>();

    for kind in [
        "create",
        "move",
        "transfer",
        "discharge",
        "return",
        "escape",
        "panic",
        "error_exit",
        "cancel",
        "opaque_boundary",
    ] {
        assert!(
            kinds.contains(kind),
            "missing lowering trace kind {kind}: {kinds:?}"
        );
    }
}

#[test]
fn lowering_trace_does_not_invent_anchor_from_same_binding() {
    let file_id = FileId(1);
    let mapped_span = kobo_ir::KoboSpan::new(10, 20, file_id);
    let unmapped_span = kobo_ir::KoboSpan::new(40, 50, file_id);
    let source_map = wrap_source_map(
        Path::new("src/main.kobo"),
        Path::new("src/main.rs"),
        vec![SourceMapEntry {
            id: "map-0".to_owned(),
            core_event_id: None,
            binding_name: "delivery".to_owned(),
            kobo_span: mapped_span,
            rs_span: RsSpan {
                line: 1,
                column_start: 1,
                column_end: 8,
            },
            ownership_tier: "plain".to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: None,
        }],
    );
    let program = ScenarioProgram {
        file_id,
        target: "trace_case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: unmapped_span,
            kind: ScenarioOpKind::Discharge {
                binding: "delivery".to_owned(),
                action: "ack".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    let trace = build_lowering_trace(&[program], &source_map);

    assert!(
        trace.is_empty(),
        "unmapped proof events must not reuse a same-binding anchor: {trace:?}"
    );
}

#[test]
fn map_entry_without_core_event_id_does_not_anchor_generated_trace() {
    let file_id = FileId(1);
    let span = kobo_ir::KoboSpan::new(10, 20, file_id);
    let source_map = wrap_source_map(
        Path::new("src/main.kobo"),
        Path::new("src/main.rs"),
        vec![SourceMapEntry {
            id: "map-0".to_owned(),
            core_event_id: None,
            binding_name: "delivery".to_owned(),
            kobo_span: span,
            rs_span: RsSpan {
                line: 1,
                column_start: 1,
                column_end: 8,
            },
            ownership_tier: "plain".to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: None,
        }],
    );
    let program = ScenarioProgram {
        file_id,
        target: "trace_case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span,
            kind: ScenarioOpKind::Discharge {
                binding: "delivery".to_owned(),
                action: "ack".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    let trace = build_lowering_trace(&[program], &source_map);

    assert!(
        trace.is_empty(),
        "generated proof trace must require an anchor with matching core_event_id: {trace:?}"
    );
}

#[test]
fn lowering_trace_core_event_ids_are_function_scoped() {
    let file_id = FileId(1);
    let first_span = kobo_ir::KoboSpan::new(10, 20, file_id);
    let second_span = kobo_ir::KoboSpan::new(40, 50, file_id);
    let source_map = wrap_source_map(
        Path::new("src/main.kobo"),
        Path::new("src/main.rs"),
        vec![
            SourceMapEntry {
                id: "proof-map-first-0".to_owned(),
                core_event_id: Some("core-first-stmt-0".to_owned()),
                binding_name: "delivery".to_owned(),
                kobo_span: first_span,
                rs_span: RsSpan {
                    line: 1,
                    column_start: 1,
                    column_end: 8,
                },
                ownership_tier: "proof-event".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            },
            SourceMapEntry {
                id: "proof-map-second-0".to_owned(),
                core_event_id: Some("core-second-stmt-0".to_owned()),
                binding_name: "delivery".to_owned(),
                kobo_span: second_span,
                rs_span: RsSpan {
                    line: 2,
                    column_start: 1,
                    column_end: 8,
                },
                ownership_tier: "proof-event".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            },
        ],
    );
    let first_program = single_discharge_program(file_id, "first", first_span);
    let second_program = single_discharge_program(file_id, "second", second_span);

    let trace = build_lowering_trace(&[first_program, second_program], &source_map);
    let core_event_ids = trace
        .iter()
        .map(|event| event.core_event_id.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        trace.len(),
        2,
        "both functions need distinct anchors: {trace:?}"
    );
    assert!(
        core_event_ids.contains("core-first-stmt-0"),
        "first function event id must be function-scoped: {trace:?}"
    );
    assert!(
        core_event_ids.contains("core-second-stmt-0"),
        "second function event id must be function-scoped: {trace:?}"
    );
}

#[test]
fn source_map_without_event_site_anchor_omits_generated_trace_event() {
    let file_id = FileId(1);
    let mapped_span = kobo_ir::KoboSpan::new(10, 20, file_id);
    let event_span = kobo_ir::KoboSpan::new(40, 50, file_id);
    let entries = vec![SourceMapEntry {
        id: "map-0".to_owned(),
        core_event_id: None,
        binding_name: "delivery".to_owned(),
        kobo_span: mapped_span,
        rs_span: RsSpan {
            line: 1,
            column_start: 1,
            column_end: 8,
        },
        ownership_tier: "plain".to_owned(),
        solver_outcome: None,
        decision_source: None,
        solver_node_id: None,
    }];
    let program = ScenarioProgram {
        file_id,
        target: "trace_case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: event_span,
            kind: ScenarioOpKind::Discharge {
                binding: "delivery".to_owned(),
                action: "ack".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    assert!(
        entries
            .iter()
            .all(|entry| !entry.id.starts_with("trace-map-")),
        "unmapped proof events must not synthesize copied trace-map anchors: {entries:?}"
    );
    let source_map = wrap_source_map(
        Path::new("src/main.kobo"),
        Path::new("src/main.rs"),
        entries,
    );
    let trace = build_lowering_trace(&[program], &source_map);

    assert!(
        trace.is_empty(),
        "unmapped proof events must not produce generated trace events: {trace:?}"
    );
}

#[test]
fn synthetic_return_is_omitted_when_no_obligation_trace_exists() {
    let file_id = FileId(1);
    let span = kobo_ir::KoboSpan::new(10, 20, file_id);
    let operations = vec![
        terminator(span, ScenarioCoreTerminatorKind::ErrorExit),
        ScenarioOp {
            span,
            kind: ScenarioOpKind::Return,
        },
    ];
    let entries = operations
        .iter()
        .enumerate()
        .map(|(order, operation)| SourceMapEntry {
            id: format!("proof-map-trace_case-{order}"),
            core_event_id: Some(super::core_event_id_for_operation(
                "trace_case",
                order,
                &operation.kind,
            )),
            binding_name: "proof".to_owned(),
            kobo_span: span,
            rs_span: RsSpan {
                line: order + 1,
                column_start: 1,
                column_end: 8,
            },
            ownership_tier: "proof-event".to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: None,
        })
        .collect();
    let source_map = wrap_source_map(
        Path::new("src/main.kobo"),
        Path::new("src/main.rs"),
        entries,
    );
    let program = ScenarioProgram {
        file_id,
        target: "trace_case".to_owned(),
        source_hash: "source".to_owned(),
        operations,
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    let trace = build_lowering_trace(&[program], &source_map);

    assert_eq!(
            trace
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["error_exit"],
            "synthetic function-end return should not create an unmatched lowering trace event: {trace:?}"
        );
}

#[test]
fn proof_event_anchors_use_generated_function_scope_not_global_snippet() {
    let source = r#"
fn fallible() -> Result<(), ()> { Ok(()) }

fn first() -> Result<(), ()> {
    fallible()?;
    Ok(())
}

fn second() -> Result<(), ()> {
    fallible()?;
    Ok(())
}
"#;
    let operation_span = span_for(source, "fn second", "fallible()?");
    let rs_source = r#"fn fallible() -> Result<(), ()> {
    Ok(())
}
fn first() -> Result<(), ()> {
    {
        fallible()
    }?;
    Ok(())
}
fn second() -> Result<(), ()> {
    {
        fallible()
    }?;
    Ok(())
}
"#;
    let mut entries = Vec::new();
    let program = ScenarioProgram {
        file_id: FileId(1),
        target: "second".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: operation_span,
            kind: ScenarioOpKind::CoreTerminator {
                kind: ScenarioCoreTerminatorKind::ErrorExit,
                boundary: None,
                policy: None,
                edges: vec!["error_exit".to_owned()],
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

    let second_function_line = line_containing(rs_source, "fn second");
    let entry = entries
        .iter()
        .find(|entry| entry.id == "proof-map-second-0")
        .expect("proof event should get a generated anchor");
    assert!(
        entry.rs_span.line > second_function_line,
        "proof anchor must be in second function, not a global text match: {entry:?}"
    );
}

#[test]
fn helper_inlined_events_can_anchor_to_helper_body() {
    let rs_source = r#"fn helper_reordered(token: Delivery) {
    token.requeue();
}

fn helper_case() {
    let delivery = Delivery {};
    helper_reordered(delivery);
}
"#;
    let mut entries = Vec::new();
    let program = ScenarioProgram {
        file_id: FileId(1),
        target: "helper_case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: kobo_ir::KoboSpan::new(20, 35, FileId(1)),
            kind: ScenarioOpKind::Discharge {
                binding: "delivery".to_owned(),
                action: "requeue".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

    let helper_line = line_containing(rs_source, "token.requeue()");
    let entry = entries
        .iter()
        .find(|entry| entry.id == "proof-map-helper_case-0")
        .expect("helper-body proof event should get a generated anchor");
    assert_eq!(
        entry.rs_span.line, helper_line,
        "helper summary discharge should anchor to the helper body: {entry:?}"
    );
}

#[test]
fn helper_body_anchors_can_be_reused_for_inlined_caller_events() {
    let rs_source = r#"fn helper_reordered(token: Delivery) {
    token.requeue();
}

fn helper_case() {
    helper_reordered(Delivery {});
}
"#;
    let mut entries = Vec::new();
    let helper_program = ScenarioProgram {
        file_id: FileId(1),
        target: "helper_reordered".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![
            ScenarioOp {
                span: kobo_ir::KoboSpan::new(1, 10, FileId(1)),
                kind: ScenarioOpKind::CreateObligation {
                    binding: "token".to_owned(),
                    type_name: "Delivery".to_owned(),
                    actions: vec!["requeue".to_owned()],
                    template: None,
                },
            },
            ScenarioOp {
                span: kobo_ir::KoboSpan::new(11, 20, FileId(1)),
                kind: ScenarioOpKind::Discharge {
                    binding: "token".to_owned(),
                    action: "requeue".to_owned(),
                },
            },
        ],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };
    let caller_program = ScenarioProgram {
        file_id: FileId(1),
        target: "helper_case".to_owned(),
        source_hash: "source".to_owned(),
        operations: helper_program.operations.clone(),
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    super::add_proof_event_source_entries(
        &mut entries,
        rs_source,
        &[helper_program, caller_program],
    );

    assert!(
        entries
            .iter()
            .any(|entry| entry.id == "proof-map-helper_case-0"
                && entry.core_event_id.as_deref() == Some("core-helper_case-stmt-0")),
        "inlined caller create should reuse the helper parameter anchor: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.id == "proof-map-helper_case-1"
                && entry.core_event_id.as_deref() == Some("core-helper_case-stmt-1")),
        "inlined caller discharge should reuse the helper body anchor: {entries:?}"
    );
}

#[test]
fn return_expression_discharge_gets_generated_anchor() {
    let rs_source = r#"fn case() -> Delivery {
    let returned = Delivery {};
    return returned;
}
"#;
    let mut entries = Vec::new();
    let program = ScenarioProgram {
        file_id: FileId(1),
        target: "case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: kobo_ir::KoboSpan::new(20, 35, FileId(1)),
            kind: ScenarioOpKind::Discharge {
                binding: "returned".to_owned(),
                action: "return".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

    let return_line = line_containing(rs_source, "return returned");
    let entry = entries
        .iter()
        .find(|entry| entry.id == "proof-map-case-0")
        .expect("return discharge should get a generated anchor");
    assert_eq!(
        entry.rs_span.line, return_line,
        "returning an obligation should anchor the discharge to the return expression: {entry:?}"
    );
}

#[test]
fn reasoned_suppression_discharge_gets_generated_anchor() {
    let rs_source = r#"fn case() {
    let suppressed = Delivery {};
}
"#;
    let mut entries = Vec::new();
    let program = ScenarioProgram {
        file_id: FileId(1),
        target: "case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: kobo_ir::KoboSpan::new(20, 35, FileId(1)),
            kind: ScenarioOpKind::Discharge {
                binding: "suppressed".to_owned(),
                action: "suppressed:tracked elsewhere".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

    let suppression_line = line_containing(rs_source, "let suppressed");
    let entry = entries
        .iter()
        .find(|entry| entry.id == "proof-map-case-0")
        .expect("reasoned suppression discharge should get a generated anchor");
    assert_eq!(
        entry.rs_span.line, suppression_line,
        "reasoned suppression should anchor the discharge to the annotated local: {entry:?}"
    );
}

#[test]
fn discharge_anchors_match_action_not_any_same_binding_method() {
    let rs_source = r#"fn case() {
    delivery.inspect();
    delivery.ack();
}
"#;
    let mut entries = Vec::new();
    let program = ScenarioProgram {
        file_id: FileId(1),
        target: "case".to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span: kobo_ir::KoboSpan::new(20, 34, FileId(1)),
            kind: ScenarioOpKind::Discharge {
                binding: "delivery".to_owned(),
                action: "ack".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    };

    super::add_proof_event_source_entries(&mut entries, rs_source, &[program]);

    let ack_line = line_containing(rs_source, "delivery.ack()");
    let entry = entries
        .iter()
        .find(|entry| entry.id == "proof-map-case-0")
        .expect("ack discharge should get a generated anchor");
    assert_eq!(
        entry.rs_span.line, ack_line,
        "ack discharge must anchor to ack(), not an earlier same-binding method: {entry:?}"
    );
}

fn trace_kind_operations(span: kobo_ir::KoboSpan) -> Vec<ScenarioOp> {
    let binding = "delivery".to_owned();
    vec![
        ScenarioOp {
            span,
            kind: ScenarioOpKind::CreateObligation {
                binding: binding.clone(),
                type_name: "Delivery".to_owned(),
                actions: vec!["ack".to_owned()],
                template: Some(ScenarioLifecycleTemplate::inferred(
                    "queue_delivery",
                    "queue_delivery",
                )),
            },
        },
        ScenarioOp {
            span,
            kind: ScenarioOpKind::MoveBinding {
                binding: binding.clone(),
            },
        },
        ScenarioOp {
            span,
            kind: ScenarioOpKind::Transfer {
                binding: binding.clone(),
                callee: "handoff".to_owned(),
                proven: true,
            },
        },
        ScenarioOp {
            span,
            kind: ScenarioOpKind::Discharge {
                binding,
                action: "ack".to_owned(),
            },
        },
        ScenarioOp {
            span,
            kind: ScenarioOpKind::ExternalBoundary {
                crate_name: "boundary".to_owned(),
                call_path: Some("boundary::call".to_owned()),
                call_arguments: Vec::<ScenarioBoundaryCallArgument>::new(),
                return_type: None,
                call_shape: ScenarioExternalCallShape::FreeFunction,
                policy: ScenarioBoundaryPolicy::Opaque,
                reason: None,
            },
        },
        terminator(span, ScenarioCoreTerminatorKind::Return),
        terminator(span, ScenarioCoreTerminatorKind::ErrorExit),
        terminator(span, ScenarioCoreTerminatorKind::Panic),
        terminator(span, ScenarioCoreTerminatorKind::Await),
        terminator(span, ScenarioCoreTerminatorKind::OpaqueBoundary),
    ]
}

fn terminator(span: kobo_ir::KoboSpan, kind: ScenarioCoreTerminatorKind) -> ScenarioOp {
    ScenarioOp {
        span,
        kind: ScenarioOpKind::CoreTerminator {
            kind,
            boundary: None,
            policy: None,
            edges: Vec::new(),
        },
    }
}

fn single_discharge_program(
    file_id: FileId,
    target: &str,
    span: kobo_ir::KoboSpan,
) -> ScenarioProgram {
    ScenarioProgram {
        file_id,
        target: target.to_owned(),
        source_hash: "source".to_owned(),
        operations: vec![ScenarioOp {
            span,
            kind: ScenarioOpKind::Discharge {
                binding: "delivery".to_owned(),
                action: "ack".to_owned(),
            },
        }],
        boundaries: Vec::new(),
        coverage: ScenarioCoverageFacts::default(),
    }
}

fn span_for(source: &str, function_needle: &str, event_needle: &str) -> kobo_ir::KoboSpan {
    let function_start = source
        .find(function_needle)
        .expect("function should exist in source");
    let event_start = source[function_start..]
        .find(event_needle)
        .map(|offset| function_start + offset)
        .expect("event should exist in source");
    kobo_ir::KoboSpan::new(
        event_start as u32,
        (event_start + event_needle.len()) as u32,
        FileId(1),
    )
}

fn line_containing(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .expect("line should exist")
}
