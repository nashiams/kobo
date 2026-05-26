use kobo_proof::{
    bounded_wording, certificate_material_hash, classify_bounded_completeness, core_material_hash,
    derive_translation_validation, normalized_bound_hash, parse_certificate_json, stable_hash,
    template_schema_hash, verify_certificate, ArtifactKind, AsyncModelEvidence, BoundDeclaration,
    BoundDimension, BoundSource, BoundedClassificationInput, BoundedCompleteness,
    BoundedHistoryEvidence, BoundedProofEvidence, CoreCfgEdge, CoreCfgNode, CoreEvidence,
    CoreTraceEvent, FunctionSummary, HashEvidence, ObligationEvent, ObligationEventKind,
    ObligationState, ObligationStatus, ProofCertificate, ReplayGrade, SourceEvidence, SourceSpan,
    TemplateSchemaEvidence, TraceEventKind, TranslationValidationInput,
    TranslationValidationStatus, VerificationContext, VerificationError,
};
use serde_json::Value;

const SOURCE: &str = "fn proof_case() { let delivery = Delivery {}; delivery.ack(); }";

fn span() -> SourceSpan {
    SourceSpan {
        path: "src/main.kobo".to_owned(),
        line: 1,
        start: 0,
        end: SOURCE.len(),
        mapped: true,
        snippet: SOURCE.to_owned(),
    }
}

fn valid_certificate() -> ProofCertificate {
    let cfg_nodes = vec![
        CoreCfgNode {
            id: "bb0".to_owned(),
            function: "proof_case".to_owned(),
            source_span: span(),
        },
        CoreCfgNode {
            id: "bb1".to_owned(),
            function: "proof_case".to_owned(),
            source_span: span(),
        },
    ];
    let cfg_edges = vec![CoreCfgEdge {
        id: "edge-proof_case-bb0-bb1".to_owned(),
        function: "proof_case".to_owned(),
        from: "bb0".to_owned(),
        to: "bb1".to_owned(),
        kind: "goto".to_owned(),
        loop_id: None,
        loop_label: None,
        loop_edge_kind: None,
        loop_entry_block: None,
        source_span: span(),
    }];
    let template_schema = TemplateSchemaEvidence {
        id: "declared_must_call:Delivery".to_owned(),
        kind: "declared_must_call".to_owned(),
        template_schema: "lifecycle-template".to_owned(),
        schema_version: 1,
        confidence: "declared_contract".to_owned(),
        source: "declaration".to_owned(),
        lifecycle_owner: "Delivery".to_owned(),
        cancel_policy: "declared_terminal_action".to_owned(),
        registry_source: "declaration".to_owned(),
        source_span: span(),
    };
    let template_hash = template_schema_hash(&template_schema).unwrap();
    let entry_env = Vec::new();
    let exit_env = vec![ObligationState {
        binding: "delivery".to_owned(),
        state: ObligationStatus::Resolved,
    }];
    let obligation_events = vec![
        ObligationEvent {
            id: "stmt-0".to_owned(),
            kind: ObligationEventKind::Create,
            binding: Some("delivery".to_owned()),
            action: None,
            loop_regions: Vec::new(),
            source_span: span(),
            state_before: Vec::new(),
            state_after: vec![ObligationState {
                binding: "delivery".to_owned(),
                state: ObligationStatus::Owned,
            }],
        },
        ObligationEvent {
            id: "stmt-1".to_owned(),
            kind: ObligationEventKind::Discharge,
            binding: Some("delivery".to_owned()),
            action: Some("ack".to_owned()),
            loop_regions: Vec::new(),
            source_span: span(),
            state_before: vec![ObligationState {
                binding: "delivery".to_owned(),
                state: ObligationStatus::Owned,
            }],
            state_after: exit_env.clone(),
        },
    ];
    let mut certificate = ProofCertificate {
        schema_version: 2,
        proof_target_version: "kobo-core-obligation-flow-1".to_owned(),
        semantic_schema: ".kproof".to_owned(),
        artifact_kind: ArtifactKind::Kproof,
        claim_scope: "modeled_core_obligation_flow_only".to_owned(),
        compiler_version: "0.1.0".to_owned(),
        source: SourceEvidence {
            path: "src/main.kobo".to_owned(),
            hash: stable_hash(SOURCE),
        },
        core: CoreEvidence {
            hash: core_material_hash(
                "core-1",
                &cfg_nodes,
                &cfg_edges,
                &[],
                &[],
                &AsyncModelEvidence::default(),
            )
            .unwrap(),
            version: "core-1".to_owned(),
            cfg_nodes,
            cfg_edges,
            loop_facts: Vec::new(),
            loop_exit_facts: Vec::new(),
            async_model: AsyncModelEvidence::default(),
        },
        replay_grade: ReplayGrade::Partial,
        template_hashes: vec![HashEvidence {
            id: template_schema.id.clone(),
            hash: template_hash,
        }],
        template_schemas: vec![template_schema],
        boundary_assumption_hashes: Vec::new(),
        boundary_assumptions: Vec::new(),
        adapter_confidence: Vec::new(),
        obligation_events,
        entry_env,
        exit_env: exit_env.clone(),
        function_summaries: vec![FunctionSummary {
            function: "proof_case".to_owned(),
            event_count: 2,
            entry_env: Vec::new(),
            exit_env,
        }],
        coverage_loss: Vec::new(),
        loop_invariants: Vec::new(),
        bounded_evidence: Vec::new(),
        core_obligation_trace: Vec::new(),
        generated_rust_trace: Vec::new(),
        trace_hashes: Vec::new(),
        translation_validation: Default::default(),
        opaque_edge_ledger: Vec::new(),
        candidate_admission: Vec::new(),
        certificate_material_hash: String::new(),
    };
    certificate.certificate_material_hash = certificate_material_hash(&certificate).unwrap();
    certificate
}

fn certificate_with_single_event_kind(kind: ObligationEventKind) -> ProofCertificate {
    let mut certificate = valid_certificate();
    certificate.entry_env = Vec::new();
    certificate.exit_env = Vec::new();
    certificate.obligation_events = vec![ObligationEvent {
        id: "stmt-0".to_owned(),
        kind,
        binding: None,
        action: None,
        loop_regions: Vec::new(),
        source_span: span(),
        state_before: Vec::new(),
        state_after: Vec::new(),
    }];
    certificate.function_summaries = vec![FunctionSummary {
        function: "proof_case".to_owned(),
        event_count: 1,
        entry_env: Vec::new(),
        exit_env: Vec::new(),
    }];
    rehash(&mut certificate);
    certificate
}

fn bounded_history(
    id: &str,
    scheduler: &str,
    fault: &str,
    cancellation: &str,
) -> BoundedHistoryEvidence {
    let queue_capacity = 1;
    let message_count = 1;
    let retry_attempts = 1;
    let timeout_path = 1;
    let loop_iteration = 1;
    let external_boundary_recording = 0;
    let material = format!(
        "{id}:{scheduler}:{fault}:{cancellation}:{loop_iteration}:{queue_capacity}:{message_count}:{retry_attempts}:{timeout_path}:{external_boundary_recording}"
    );
    BoundedHistoryEvidence {
        id: id.to_owned(),
        scheduler: scheduler.to_owned(),
        fault: fault.to_owned(),
        cancellation: cancellation.to_owned(),
        loop_iteration,
        queue_capacity,
        message_count,
        retry_attempts,
        timeout_path,
        external_boundary_recording,
        history_hash: stable_hash(&material),
    }
}

fn complete_bounded_evidence() -> BoundedProofEvidence {
    let mut evidence = BoundedProofEvidence {
        id: "bounded-proof_case-0".to_owned(),
        function: "proof_case".to_owned(),
        loop_ids: Vec::new(),
        bounds: vec![
            BoundDeclaration {
                dimension: BoundDimension::SchedulerHistories,
                value: 4,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::LoopIterations,
                value: 1,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::FaultInjectionChoices,
                value: 2,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::CancellationPoints,
                value: 1,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::QueueCapacity,
                value: 1,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::MessageCount,
                value: 1,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::RetryAttempts,
                value: 1,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::TimeoutPaths,
                value: 1,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
            BoundDeclaration {
                dimension: BoundDimension::ExternalBoundaryRecordings,
                value: 0,
                source: BoundSource::Ward,
                proof_relevant: true,
            },
        ],
        normalized_bound_hash: String::new(),
        enumerated_history_count: 4,
        expected_complete_history_count: Some(4),
        scheduler_dimensions: vec!["fifo".to_owned(), "round_robin".to_owned()],
        fault_dimensions: vec!["none".to_owned(), "timeout".to_owned()],
        cancellation_points: vec!["none".to_owned()],
        canonical_histories: vec![
            bounded_history("history-proof_case-0", "fifo", "none", "none"),
            bounded_history("history-proof_case-1", "round_robin", "none", "none"),
            bounded_history("history-proof_case-2", "fifo", "timeout", "none"),
            bounded_history("history-proof_case-3", "round_robin", "timeout", "none"),
        ],
        pruned_histories: Vec::new(),
        completeness: BoundedCompleteness::Complete,
        wording: "bounded proof: all 4 histories explored under declared bounds".to_owned(),
    };
    evidence.normalized_bound_hash = normalized_bound_hash(&evidence);
    evidence
}

fn context() -> VerificationContext {
    VerificationContext {
        source: SOURCE.to_owned(),
        source_map: None,
    }
}

fn context_with_source_map(source_map: serde_json::Value) -> VerificationContext {
    VerificationContext {
        source: SOURCE.to_owned(),
        source_map: Some(serde_json::to_string(&source_map).unwrap()),
    }
}

fn single_create_source_map() -> serde_json::Value {
    let kobo_span = serde_json::json!({
        "start": 0,
        "end": SOURCE.len(),
        "file_id": 0
    });
    let rs_span = serde_json::json!({
        "line": 1,
        "column_start": 0,
        "column_end": 8
    });
    serde_json::json!({
        "version": 3,
        "file": "src/main.rs",
        "sources": ["src/main.kobo"],
        "x_kobo_mappings": [
            {
                "id": "map-0",
                "core_event_id": "core-create-delivery",
                "binding_name": "delivery",
                "rs_span": rs_span,
                "kobo_span": kobo_span,
                "ownership_tier": "linear"
            }
        ],
        "lowering_trace": [
            {
                "id": "lowering-proof_case-0",
                "core_event_id": "core-create-delivery",
                "function": "proof_case",
                "kind": "create",
                "binding": "delivery",
                "order": 0,
                "source_map_entry_id": "map-0",
                "rs_span": rs_span,
                "kobo_span": kobo_span,
                "lowering_phase": "obligation_lowering",
                "template_id": "queue_delivery",
                "template_version": "0.1"
            }
        ]
    })
}

#[test]
fn proof_crate_derives_translation_status_and_mismatches() {
    let core_trace = vec![CoreTraceEvent {
        id: "core-discharge-delivery".to_owned(),
        kind: TraceEventKind::Discharge,
        binding: Some("delivery".to_owned()),
        order: 1,
        source_span: span(),
        template_id: Some("queue_delivery".to_owned()),
        template_version: Some("0.1".to_owned()),
    }];
    let evidence = derive_translation_validation(TranslationValidationInput {
        core_trace: &core_trace,
        generated_trace: &[],
        has_source_map: true,
    });

    assert_eq!(evidence.status, TranslationValidationStatus::Failed);
    assert!(
        evidence
            .mismatches
            .iter()
            .any(|mismatch| mismatch.core_event_id.as_deref() == Some("core-discharge-delivery")),
        "missing generated event should be proof-crate-classified: {evidence:?}"
    );
}

#[test]
fn proof_crate_classifies_bounded_completeness_and_wording() {
    let input = BoundedClassificationInput {
        declared: BoundedCompleteness::Complete,
        declared_expected: Some(2),
        is_complete: true,
        expected_complete_history_count: 2,
        enumerated_history_count: 1,
        has_scheduler_dimensions: true,
        has_fault_dimensions: true,
        has_cancellation_points: true,
        has_required_dimensions: true,
    };

    let completeness = classify_bounded_completeness(&input);
    let wording = bounded_wording(&completeness, 1, Some(2));

    assert_eq!(completeness, BoundedCompleteness::Incomplete);
    assert!(
        wording.contains("evidence only"),
        "incomplete proof must be proof-crate-classified as evidence-only: {wording}"
    );
}

fn rehash(certificate: &mut ProofCertificate) {
    certificate.certificate_material_hash = certificate_material_hash(certificate).unwrap();
}

fn mutate_json(mut certificate: ProofCertificate, mutate: impl FnOnce(&mut Value)) -> String {
    rehash(&mut certificate);
    let mut value = serde_json::to_value(certificate).unwrap();
    mutate(&mut value);
    serde_json::to_string(&value).unwrap()
}

fn assert_parse_field_error(source: String, expected_field: &str) {
    let error = parse_certificate_json(&source).unwrap_err();
    assert!(
        matches!(
            error,
            VerificationError::UnsupportedCertificateField { ref field, .. }
                if field == expected_field
        ),
        "expected unsupported field `{expected_field}`, got {error:?}"
    );
}

#[test]
fn valid_certificate_verifies() {
    let report = verify_certificate(&valid_certificate(), &context())
        .expect("valid certificate should verify");

    assert_eq!(report.source_hash, stable_hash(SOURCE));
    assert_eq!(report.checked_obligation_events, 2);
}

#[test]
fn missing_source_hash_rejected() {
    let mut certificate = valid_certificate();
    certificate.source.hash.clear();

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert_eq!(error, VerificationError::MissingSourceHash);
}

#[test]
fn core_hash_mismatch_rejected() {
    let mut certificate = valid_certificate();
    certificate.core.hash = "bad-core-hash".to_owned();

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(error, VerificationError::CoreHashMismatch { .. }));
}

#[test]
fn unsupported_certificate_schema_rejected_before_hash_replay() {
    let mut certificate = valid_certificate();
    certificate.schema_version = 99;
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateHeader { ref field, .. }
            if field == "schema_version"
    ));
}

#[test]
fn unsupported_claim_scope_rejected_even_with_matching_material_hash() {
    let mut certificate = valid_certificate();
    certificate.claim_scope = "whole_program_total_correctness".to_owned();
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateHeader { ref field, .. }
            if field == "claim_scope"
    ));
}

#[test]
fn unknown_event_kind_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        value["obligation_events"][0]["kind"] = Value::String("teleport".to_owned());
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(error, VerificationError::UnknownEventKind { .. }));
}

#[test]
fn invalid_artifact_kind_reports_header_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["artifact_kind"] = Value::String("totally_proof".to_owned());
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateHeader { ref field, .. }
            if field == "artifact_kind"
    ));
}

#[test]
fn invalid_replay_grade_reports_replay_grade_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["replay_grade"] = Value::String("mystery".to_owned());
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateField { ref field, .. }
            if field == "replay_grade"
    ));
}

#[test]
fn invalid_adapter_confidence_reports_confidence_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["adapter_confidence"] = serde_json::json!([{
            "boundary": "runtime",
            "adapter": "tokio",
            "version": null,
            "confidence": "mystery",
            "replay_grade": "partial",
            "outcome": "modeled",
            "reason": "fixture"
        }]);
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateField { ref field, .. }
            if field == "adapter_confidence[0].confidence"
    ));
}

#[test]
fn invalid_adapter_replay_grade_reports_adapter_replay_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["adapter_confidence"] = serde_json::json!([{
            "boundary": "runtime",
            "adapter": "tokio",
            "version": null,
            "confidence": "modeled",
            "replay_grade": "mystery",
            "outcome": "modeled",
            "reason": "fixture"
        }]);
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateField { ref field, .. }
            if field == "adapter_confidence[0].replay_grade"
    ));
}

#[test]
fn invalid_candidate_replay_grade_reports_candidate_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["candidate_admission"] = serde_json::json!([{
            "replay_grade": "mystery"
        }]);
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedCertificateField { ref field, .. }
            if field == "candidate_admission[0].replay_grade"
    ));
}

#[test]
fn extended_obligation_event_kinds_parse_and_verify() {
    for kind in [
        ObligationEventKind::Escape,
        ObligationEventKind::UnsupportedContainer,
        ObligationEventKind::Call,
    ] {
        let certificate = certificate_with_single_event_kind(kind);
        let source = serde_json::to_string(&certificate).expect("certificate should render");
        let parsed = parse_certificate_json(&source).expect("valid event kind should parse");
        let report =
            verify_certificate(&parsed, &context()).expect("valid event kind should verify");

        assert_eq!(report.checked_obligation_events, 1);
    }
}

#[test]
fn invalid_entry_env_status_reports_state_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["entry_env"] = serde_json::json!([{
            "binding": "delivery",
            "state": "mystery"
        }]);
    });

    assert_parse_field_error(source, "entry_env[0].state");
}

#[test]
fn invalid_event_state_after_status_reports_state_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["obligation_events"][0]["state_after"][0]["state"] =
            Value::String("mystery".to_owned());
    });

    assert_parse_field_error(source, "obligation_events[0].state_after[0].state");
}

#[test]
fn invalid_future_state_obligation_status_reports_state_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["core"]["async_model"]["future_state_obligations"] = serde_json::json!([{
            "binding": "delivery",
            "state": "mystery",
            "suspension_state": "s0",
            "source_span": span()
        }]);
    });

    assert_parse_field_error(source, "core.async_model.future_state_obligations[0].state");
}

#[test]
fn invalid_select_path_status_reports_state_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["core"]["async_model"]["select_paths"] = serde_json::json!([{
            "id": "select-0",
            "function": "proof_case",
            "branch_block": "bb0",
            "branch_target": "bb1",
            "path_kind": "selected",
            "obligation_results": [{
                "binding": "delivery",
                "state": "mystery"
            }],
            "cancelled_obligations": [],
            "obligation_result_hash": "hash",
            "source_span": span()
        }]);
    });

    assert_parse_field_error(
        source,
        "core.async_model.select_paths[0].obligation_results[0].state",
    );
}

#[test]
fn invalid_function_summary_status_reports_state_field() {
    let source = mutate_json(valid_certificate(), |value| {
        value["function_summaries"][0]["exit_env"][0]["state"] =
            Value::String("mystery".to_owned());
    });

    assert_parse_field_error(source, "function_summaries[0].exit_env[0].state");
}

#[test]
fn missing_template_schema_fields_are_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        let template = value["template_schemas"][0]
            .as_object_mut()
            .expect("template schema entry should be an object");
        template.remove("template_schema");
        template.remove("schema_version");
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(
        error.to_string().contains("missing field"),
        "missing public schema fields should not be silently defaulted: {error}"
    );
}

#[test]
fn unsupported_template_schema_rejected() {
    let mut certificate = valid_certificate();
    certificate.template_schemas[0].schema_version = 99;

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedTemplateSchema { .. }
    ));
}

#[test]
fn metadata_only_template_schema_rejected() {
    let mut certificate = valid_certificate();
    certificate.template_schemas[0].source = "adapter".to_owned();
    certificate.template_schemas[0].confidence = "metadata-only".to_owned();
    certificate.template_hashes[0].hash =
        template_schema_hash(&certificate.template_schemas[0]).unwrap();
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnsupportedTemplateSchema { .. }
    ));
}

#[test]
fn unrecognized_boundary_policy_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        value["boundary_assumptions"] = serde_json::json!([{
            "id": "boundary-0",
            "boundary": "payments",
            "policy": "mystery",
            "reason": null,
            "source_span": span()
        }]);
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::UnknownBoundaryPolicy { .. }
    ));
}

#[test]
fn opaque_edge_without_ledger_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        value["boundary_assumptions"] = serde_json::json!([{
            "id": "boundary-0",
            "boundary": "filesystem",
            "policy": "opaque",
            "reason": "opaque fixture",
            "source_span": span()
        }]);
        value["opaque_edge_ledger"] = serde_json::json!([]);
    });
    let certificate = parse_certificate_json(&source).unwrap();

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::OpaqueEdgeWithoutLedger { .. }
    ));
}

#[test]
fn exact_replay_with_debt_boundary_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        value["replay_grade"] = Value::String("exact".to_owned());
        value["boundary_assumptions"] = serde_json::json!([{
            "id": "boundary-0",
            "boundary": "payments",
            "policy": "debt",
            "reason": "unresolved adapter",
            "source_span": span()
        }]);
    });
    let certificate = parse_certificate_json(&source).unwrap();

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::ExactReplayWithDebtBoundary { .. }
    ));
}

#[test]
fn unknown_unversioned_field_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        value["future_unversioned_field"] = Value::Bool(true);
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(error, VerificationError::UnknownField { .. }));
}

#[test]
fn loop_back_edge_leak_rejected_from_v15_invariant_evidence() {
    let template_schema = TemplateSchemaEvidence {
        id: "queue_delivery".to_owned(),
        kind: "queue_delivery".to_owned(),
        template_schema: "lifecycle-template".to_owned(),
        schema_version: 1,
        confidence: "exact_template".to_owned(),
        source: "built_in".to_owned(),
        lifecycle_owner: "queue".to_owned(),
        cancel_policy: "terminal_action_or_requeue".to_owned(),
        registry_source: "builtin_protocol_registry".to_owned(),
        source_span: span(),
    };
    let template_hash = template_schema_hash(&template_schema).unwrap();
    let source = mutate_json(valid_certificate(), |value| {
        value["template_schemas"] = serde_json::json!([template_schema]);
        value["template_hashes"] = serde_json::json!([{
            "id": "queue_delivery",
            "hash": template_hash,
        }]);
        value["obligation_events"][0]["loop_regions"] = serde_json::json!(["loop-proof"]);
        value["obligation_events"][1]["loop_regions"] = serde_json::json!(["loop-proof"]);
        value["loop_invariants"] = serde_json::json!([{
            "id": "loop-proof_case-0",
            "function": "proof_case",
            "loop_id": "loop-proof",
            "entry_block": "bb0",
            "back_edge_source": "bb1",
            "back_edge_target": "bb0",
            "tier": "inferred",
            "expression": "no_pending(Delivery)",
            "source_span": span(),
            "obligations_created": ["delivery"],
            "back_edge_states": [{
                "binding": "delivery",
                "state": "owned"
            }],
            "preservation": "preserved",
            "template": {
                "id": "queue_delivery",
                "version": "0.1",
                "schema_hash": template_hash,
                "source": "built_in",
                "confidence": "exact",
                "obligation_kind": "Delivery",
                "lifecycle_owner": "queue"
            },
            "binding_templates": [{
                "binding": "delivery",
                "id": "queue_delivery",
                "version": "0.1",
                "schema_hash": template_hash,
                "source": "built_in",
                "confidence": "exact",
                "obligation_kind": "Delivery",
                "lifecycle_owner": "queue"
            }],
            "downgrade_reason": null
        }]);
    });
    let certificate = parse_certificate_json(&source).expect("v15 loop fields should parse");

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::LoopBackEdgeLeak { ref loop_id, ref binding }
            if loop_id == "loop-proof_case-0" && binding == "delivery"
    ));
}

#[test]
fn incomplete_bounded_enumeration_cannot_claim_bounded_proof() {
    let mut certificate = valid_certificate();
    let mut evidence = complete_bounded_evidence();
    evidence.enumerated_history_count = 2;
    evidence.canonical_histories.truncate(2);
    evidence.completeness = BoundedCompleteness::Sampled;
    evidence.wording = "bounded proof: all 2 histories explored under declared bounds".to_owned();
    evidence.normalized_bound_hash = normalized_bound_hash(&evidence);
    certificate.bounded_evidence = vec![evidence];
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::IncompleteBoundedEnumeration { ref evidence_id, .. }
            if evidence_id == "bounded-proof_case-0"
    ));
}

#[test]
fn bounded_evidence_rejects_forged_normalized_bound_hash() {
    let mut certificate = valid_certificate();
    certificate.bounded_evidence = vec![complete_bounded_evidence()];
    certificate.bounded_evidence[0].normalized_bound_hash = "forged-bound-hash".to_owned();
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::BoundedEvidenceHashMismatch { ref evidence_id, .. }
            if evidence_id == "bounded-proof_case-0"
    ));
}

#[test]
fn complete_bounded_evidence_rejects_missing_fault_bound() {
    let mut certificate = valid_certificate();
    let mut evidence = complete_bounded_evidence();
    evidence
        .bounds
        .retain(|bound| bound.dimension != BoundDimension::FaultInjectionChoices);
    evidence.normalized_bound_hash = normalized_bound_hash(&evidence);
    certificate.bounded_evidence = vec![evidence];
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::MissingBoundedProofDimension { ref evidence_id }
            if evidence_id == "bounded-proof_case-0"
    ));
}

#[test]
fn complete_bounded_evidence_rejects_missing_queue_capacity_bound() {
    let mut certificate = valid_certificate();
    let mut evidence = complete_bounded_evidence();
    evidence
        .bounds
        .retain(|bound| bound.dimension != BoundDimension::QueueCapacity);
    evidence.normalized_bound_hash = normalized_bound_hash(&evidence);
    certificate.bounded_evidence = vec![evidence];
    rehash(&mut certificate);

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::MissingBoundedProofDimension { ref evidence_id }
            if evidence_id == "bounded-proof_case-0"
    ));
}

#[test]
fn translation_validation_rejects_dropped_discharge_event() {
    let source = mutate_json(valid_certificate(), |value| {
        value["core_obligation_trace"] = serde_json::json!([
            {
                "id": "core-create-delivery",
                "kind": "create",
                "binding": "delivery",
                "order": 0,
                "source_span": span(),
                "template_id": "queue_delivery",
                "template_version": "0.1"
            },
            {
                "id": "core-discharge-delivery",
                "kind": "discharge",
                "binding": "delivery",
                "order": 1,
                "source_span": span(),
                "template_id": "queue_delivery",
                "template_version": "0.1"
            }
        ]);
        value["generated_rust_trace"] = serde_json::json!([
            {
                "id": "generated-create-delivery",
                "core_event_id": "core-create-delivery",
                "kind": "create",
                "binding": "delivery",
                "order": 0,
                "source_map_anchor": {
                    "id": "map-0",
                    "status": "mapped",
                    "generated_span": {
                        "path": "src/main.rs",
                        "line": 1,
                        "start": 0,
                        "end": 8,
                        "mapped": true,
                        "snippet": "delivery"
                    },
                    "kobo_span": span()
                },
                "lowering_phase": "obligation_lowering",
                "template_id": "queue_delivery",
                "template_version": "0.1"
            }
        ]);
        value["translation_validation"] = serde_json::json!({
            "status": "validated",
            "mismatches": []
        });
    });
    let certificate = parse_certificate_json(&source).expect("v15 trace fields should parse");

    let error = verify_certificate(
        &certificate,
        &context_with_source_map(single_create_source_map()),
    )
    .unwrap_err();

    assert!(
        matches!(
            error,
            VerificationError::TranslationTraceMissingEvent { ref core_event_id }
                if core_event_id == "core-discharge-delivery"
        ),
        "expected dropped discharge to fail as missing generated event, got {error:?}"
    );
}
