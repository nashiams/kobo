use kobo_proof::{
    certificate_material_hash, core_material_hash, parse_certificate_json, stable_hash,
    template_schema_hash, verify_certificate, ArtifactKind, AsyncModelEvidence, CoreCfgEdge,
    CoreCfgNode, CoreEvidence, FunctionSummary, HashEvidence, ObligationEvent, ObligationEventKind,
    ObligationState, ObligationStatus, ProofCertificate, ReplayGrade, SourceEvidence, SourceSpan,
    TemplateSchemaEvidence, VerificationContext, VerificationError,
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
        source_span: span(),
    }];
    let template_schema = TemplateSchemaEvidence {
        id: "declared_must_call:Delivery".to_owned(),
        kind: "declared_must_call".to_owned(),
        template_schema: "lifecycle-template".to_owned(),
        schema_version: 1,
        confidence: "declared_contract".to_owned(),
        source: "declaration".to_owned(),
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
                &AsyncModelEvidence::default(),
            )
            .unwrap(),
            version: "core-1".to_owned(),
            cfg_nodes,
            cfg_edges,
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
        opaque_edge_ledger: Vec::new(),
        candidate_admission: Vec::new(),
        certificate_material_hash: String::new(),
    };
    certificate.certificate_material_hash = certificate_material_hash(&certificate).unwrap();
    certificate
}

fn context() -> VerificationContext {
    VerificationContext {
        source: SOURCE.to_owned(),
    }
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
