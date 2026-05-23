use kobo_proof::{
    certificate_material_hash, core_material_hash, parse_certificate_json, stable_hash,
    verify_certificate, ArtifactKind, AsyncModelEvidence, CoreCfgEdge, CoreCfgNode, CoreEvidence,
    FunctionSummary, HashEvidence, ObligationEvent, ObligationEventKind, ObligationState,
    ProofCertificate, ReplayGrade, SourceEvidence, SourceSpan, TemplateVersionEvidence,
    VerificationContext, VerificationError,
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
    let template_version = TemplateVersionEvidence {
        id: "declared_must_call:Delivery".to_owned(),
        kind: "declared_must_call".to_owned(),
        version: "v0.13.0".to_owned(),
        confidence: "declared_contract".to_owned(),
        source: "declaration".to_owned(),
        source_span: span(),
    };
    let template_hash = stable_hash(&serde_json::to_string(&template_version).unwrap());
    let entry_env = Vec::new();
    let exit_env = vec![ObligationState {
        binding: "delivery".to_owned(),
        state: "resolved".to_owned(),
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
                state: "owned".to_owned(),
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
                state: "owned".to_owned(),
            }],
            state_after: exit_env.clone(),
        },
    ];
    let mut certificate = ProofCertificate {
        schema_version: 1,
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
            hash: core_material_hash("core-1", &cfg_nodes, &cfg_edges).unwrap(),
            version: "core-1".to_owned(),
            cfg_nodes,
            cfg_edges,
            async_model: AsyncModelEvidence::default(),
        },
        replay_grade: ReplayGrade::Partial,
        template_hashes: vec![HashEvidence {
            id: template_version.id.clone(),
            hash: template_hash,
        }],
        template_versions: vec![template_version],
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

fn mutate_json(mut certificate: ProofCertificate, mutate: impl FnOnce(&mut Value)) -> String {
    certificate.certificate_material_hash = certificate_material_hash(&certificate).unwrap();
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
fn unknown_event_kind_rejected() {
    let source = mutate_json(valid_certificate(), |value| {
        value["obligation_events"][0]["kind"] = Value::String("teleport".to_owned());
    });

    let error = parse_certificate_json(&source).unwrap_err();

    assert!(matches!(error, VerificationError::UnknownEventKind { .. }));
}

#[test]
fn stale_template_version_rejected() {
    let mut certificate = valid_certificate();
    certificate.template_versions[0].version = "v0.0.0-stale".to_owned();

    let error = verify_certificate(&certificate, &context()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::StaleTemplateVersion { .. }
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
