use kobo_proof::{
    certificate_material_hash, core_material_hash, load_lean_rule_manifest,
    load_obligation_rule_catalog, stable_hash, template_schema_hash, verify_certificate,
    ArtifactKind, AsyncModelEvidence, BoundaryAssumption, BoundaryPolicy, CoreCfgEdge, CoreCfgNode,
    CoreEvidence, FunctionSummary, HashEvidence, ObligationEvent, ObligationEventKind,
    ObligationState, ObligationStatus, OpaqueLedgerEntry, ProofCertificate, ReplayGrade,
    SourceEvidence, SourceSpan, TemplateSchemaEvidence, VerificationContext, VerificationError,
};

const SOURCE: &str = "fn proof_case() { let delivery = Delivery {}; delivery.ack(); }";

#[test]
fn rule_create_records_owned_obligation() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::Transfer,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::Transferred)]),
        ),
    ]);

    let report = verify(&certificate).expect("create event should replay");

    assert_eq!(
        certificate.obligation_events[0].state_after,
        states(&[("delivery", ObligationStatus::Owned)])
    );
    assert_eq!(report.checked_obligation_events, 2);
}

#[test]
fn rule_transfer_moves_owned_obligation_to_callee() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::Transfer,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::Transferred)]),
        ),
    ]);

    let report = verify(&certificate).expect("transfer event should replay");

    assert_eq!(report.checked_obligation_events, 2);
}

#[test]
fn rule_transfer_also_models_move_projection() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::Move,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::Moved)]),
        ),
    ]);

    let error = verify(&certificate).expect_err("moved obligation must remain unresolved at exit");

    assert!(matches!(
        error,
        VerificationError::UnresolvedExitObligation { .. }
    ));
}

#[test]
fn rule_discharge_rejects_moved_obligation() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::Move,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::Moved)]),
        ),
        event(
            "stmt-2",
            ObligationEventKind::Discharge,
            states(&[("delivery", ObligationStatus::Moved)]),
            states(&[("delivery", ObligationStatus::Resolved)]),
        ),
    ]);

    let error = verify(&certificate).expect_err("discharge from moved must fail");

    assert!(matches!(
        error,
        VerificationError::ObligationReplayMismatch { .. }
    ));
}

#[test]
fn rule_split_requires_branch_state_join_evidence() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::BranchUnresolved,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::BranchUnresolved)]),
        ),
    ]);

    let error = verify(&certificate)
        .expect_err("branch-unresolved obligation must remain unresolved at exit");

    assert!(matches!(
        error,
        VerificationError::UnresolvedExitObligation { .. }
    ));
}

#[test]
fn rule_opaque_rejects_branch_unresolved_escape() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::BranchUnresolved,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::BranchUnresolved)]),
        ),
        event(
            "stmt-2",
            ObligationEventKind::Escape,
            states(&[("delivery", ObligationStatus::BranchUnresolved)]),
            states(&[("delivery", ObligationStatus::Escaped)]),
        ),
    ]);

    let error = verify(&certificate).expect_err("escape from branch-unresolved must fail");

    assert!(matches!(
        error,
        VerificationError::ObligationReplayMismatch { .. }
    ));
}

#[test]
fn rule_discharge_resolves_transferred_obligation() {
    let certificate = certificate_with_events(vec![
        event(
            "stmt-0",
            ObligationEventKind::Create,
            Vec::new(),
            states(&[("delivery", ObligationStatus::Owned)]),
        ),
        event(
            "stmt-1",
            ObligationEventKind::Transfer,
            states(&[("delivery", ObligationStatus::Owned)]),
            states(&[("delivery", ObligationStatus::Transferred)]),
        ),
        event(
            "stmt-2",
            ObligationEventKind::Discharge,
            states(&[("delivery", ObligationStatus::Transferred)]),
            states(&[("delivery", ObligationStatus::Resolved)]),
        ),
    ]);

    let report = verify(&certificate).expect("discharge event should replay");

    assert_eq!(report.checked_obligation_events, 3);
}

#[test]
fn rule_return_rejects_unresolved_local_exit() {
    let certificate = certificate_with_exit("return");
    let error = verify(&certificate).expect_err("return cannot silently lose owned obligation");

    assert!(matches!(
        error,
        VerificationError::UnresolvedExitObligation { .. }
            | VerificationError::CfgEdgeTransitionMismatch { .. }
    ));
}

#[test]
fn rule_cancel_requires_future_state_obligation_evidence() {
    let catalog = load_obligation_rule_catalog().expect("catalog should parse");
    let cancel = catalog
        .rules
        .iter()
        .find(|rule| rule.id == "cancel")
        .expect("cancel rule should exist");

    assert!(
        cancel
            .required_certificate_fields
            .iter()
            .any(|field| field == "core.async_model.future_state_obligations"),
        "cancel rule must require future-state obligation evidence"
    );
}

#[test]
fn rule_panic_rejects_unresolved_local_exit() {
    let certificate = certificate_with_exit("panic");
    let error = verify(&certificate).expect_err("panic cannot silently lose owned obligation");

    assert!(matches!(
        error,
        VerificationError::UnresolvedExitObligation { .. }
            | VerificationError::CfgEdgeTransitionMismatch { .. }
    ));
}

#[test]
fn rule_opaque_requires_ledger_entry() {
    let mut certificate = certificate_with_events(Vec::new());
    certificate.boundary_assumptions = vec![BoundaryAssumption {
        id: "boundary-opaque-0".to_owned(),
        boundary: "external.queue".to_owned(),
        policy: BoundaryPolicy::Opaque,
        reason: Some("external queue model is outside this proof".to_owned()),
        source_span: span(),
    }];
    certificate.opaque_edge_ledger = Vec::new();
    rehash(&mut certificate);

    let error = verify(&certificate).expect_err("opaque boundary without ledger must fail");

    assert!(matches!(
        error,
        VerificationError::OpaqueEdgeWithoutLedger { .. }
    ));
}

#[test]
fn rule_opaque_rejects_ledger_entry_without_cfg_edge() {
    let mut certificate = certificate_with_opaque_exit();
    certificate.opaque_edge_ledger = vec![OpaqueLedgerEntry {
        edge_id: "edge-proof_case-bb9-opaque_boundary".to_owned(),
        boundary: "external.queue".to_owned(),
        evidence_hash: stable_hash("external.queue"),
    }];
    rehash(&mut certificate);

    let error = verify(&certificate).expect_err("opaque ledger must bind to a CFG edge");

    assert!(matches!(
        error,
        VerificationError::OpaqueEdgeWithoutLedger { .. }
    ));
}

#[test]
fn rule_opaque_rejects_cfg_edge_with_only_opaque_target() {
    let mut certificate = certificate_with_opaque_exit();
    certificate.core.cfg_edges[0].kind = "goto".to_owned();
    certificate.core.cfg_edges[0].to = "opaque_boundary".to_owned();
    rehash_core(&mut certificate);
    rehash(&mut certificate);

    let error = verify(&certificate)
        .expect_err("opaque ledger must bind to a CFG edge whose kind and target are opaque");

    assert!(matches!(
        error,
        VerificationError::OpaqueEdgeWithoutLedger { .. }
    ));
}

#[test]
fn rule_catalog_matches_parsed_lean_rule_manifest() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let manifest = load_lean_rule_manifest().expect("Lean rule manifest should parse");

    for rule in &catalog.rules {
        let lean_rule = manifest
            .rule(rule.id.as_str())
            .unwrap_or_else(|| panic!("Lean manifest should contain rule `{}`", rule.id));

        assert_eq!(lean_rule.constructor, rule.lean_rule);
        assert_eq!(lean_rule.constructor_arity, rule.lean_constructor_arity);
        assert_eq!(lean_rule.required_premises, rule.lean_required_premises);
        assert_eq!(lean_rule.output_states, rule.lean_output_states);
    }
    assert_eq!(
        manifest.template_assumption_fields,
        catalog
            .rules
            .iter()
            .find(|rule| rule.id == "opaque")
            .expect("opaque rule should exist")
            .template_assumption_fields
    );
}

#[test]
fn rule_opaque_accepts_ledger_entry_for_cfg_edge() {
    let certificate = certificate_with_opaque_exit();

    verify(&certificate).expect("opaque ledger should bind to the modeled CFG edge");
}

fn certificate_with_opaque_exit() -> ProofCertificate {
    let cfg_nodes = vec![CoreCfgNode {
        id: "bb0".to_owned(),
        function: "proof_case".to_owned(),
        source_span: span(),
    }];
    let cfg_edges = vec![CoreCfgEdge {
        id: "edge-proof_case-bb0-opaque_boundary".to_owned(),
        function: "proof_case".to_owned(),
        from: "bb0".to_owned(),
        to: "opaque_boundary".to_owned(),
        kind: "opaque_boundary".to_owned(),
        loop_id: None,
        loop_label: None,
        loop_edge_kind: None,
        loop_entry_block: None,
        source_span: span(),
    }];
    let mut certificate = certificate_from_parts(cfg_nodes, cfg_edges, Vec::new(), Vec::new());
    certificate.boundary_assumptions = vec![BoundaryAssumption {
        id: "boundary-opaque-0".to_owned(),
        boundary: "external.queue".to_owned(),
        policy: BoundaryPolicy::Opaque,
        reason: Some("external queue model is outside this proof".to_owned()),
        source_span: span(),
    }];
    certificate.opaque_edge_ledger = vec![OpaqueLedgerEntry {
        edge_id: "edge-proof_case-bb0-opaque_boundary".to_owned(),
        boundary: "external.queue".to_owned(),
        evidence_hash: stable_hash("external.queue"),
    }];
    rehash(&mut certificate);
    certificate
}

fn certificate_with_exit(target: &str) -> ProofCertificate {
    let cfg_nodes = vec![CoreCfgNode {
        id: "bb0".to_owned(),
        function: "proof_case".to_owned(),
        source_span: span(),
    }];
    let cfg_edges = vec![CoreCfgEdge {
        id: format!("edge-proof_case-bb0-{target}"),
        function: "proof_case".to_owned(),
        from: "bb0".to_owned(),
        to: target.to_owned(),
        kind: target.to_owned(),
        loop_id: None,
        loop_label: None,
        loop_edge_kind: None,
        loop_entry_block: None,
        source_span: span(),
    }];
    let event = event(
        "stmt-0",
        ObligationEventKind::Create,
        Vec::new(),
        states(&[("delivery", ObligationStatus::Owned)]),
    );
    certificate_from_parts(
        cfg_nodes,
        cfg_edges,
        vec![event],
        states(&[("delivery", ObligationStatus::Owned)]),
    )
}

fn certificate_with_events(events: Vec<ObligationEvent>) -> ProofCertificate {
    let cfg_nodes = (0..events.len().max(1))
        .map(|index| CoreCfgNode {
            id: format!("bb{index}"),
            function: "proof_case".to_owned(),
            source_span: span(),
        })
        .collect::<Vec<_>>();
    let cfg_edges = (0..events.len().saturating_sub(1))
        .map(|index| CoreCfgEdge {
            id: format!("edge-proof_case-bb{index}-bb{}", index + 1),
            function: "proof_case".to_owned(),
            from: format!("bb{index}"),
            to: format!("bb{}", index + 1),
            kind: "goto".to_owned(),
            loop_id: None,
            loop_label: None,
            loop_edge_kind: None,
            loop_entry_block: None,
            source_span: span(),
        })
        .collect::<Vec<_>>();
    let exit_env = events
        .last()
        .map(|event| event.state_after.clone())
        .unwrap_or_default();
    certificate_from_parts(cfg_nodes, cfg_edges, events, exit_env)
}

fn certificate_from_parts(
    cfg_nodes: Vec<CoreCfgNode>,
    cfg_edges: Vec<CoreCfgEdge>,
    obligation_events: Vec<ObligationEvent>,
    exit_env: Vec<ObligationState>,
) -> ProofCertificate {
    let template_schema = template_schema();
    let event_count = obligation_events.len();
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
            .expect("core hash should serialize"),
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
            hash: template_schema_hash(&template_schema).expect("template hash should serialize"),
        }],
        template_schemas: vec![template_schema],
        boundary_assumption_hashes: Vec::new(),
        boundary_assumptions: Vec::new(),
        adapter_confidence: Vec::new(),
        obligation_events,
        entry_env: Vec::new(),
        exit_env: exit_env.clone(),
        function_summaries: vec![FunctionSummary {
            function: "proof_case".to_owned(),
            event_count,
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
    rehash(&mut certificate);
    certificate
}

fn event(
    id: &str,
    kind: ObligationEventKind,
    state_before: Vec<ObligationState>,
    state_after: Vec<ObligationState>,
) -> ObligationEvent {
    ObligationEvent {
        id: id.to_owned(),
        kind,
        binding: Some("delivery".to_owned()),
        action: None,
        loop_regions: Vec::new(),
        source_span: span(),
        state_before,
        state_after,
    }
}

fn states(values: &[(&str, ObligationStatus)]) -> Vec<ObligationState> {
    values
        .iter()
        .map(|(binding, state)| ObligationState {
            binding: (*binding).to_owned(),
            state: state.clone(),
        })
        .collect()
}

fn template_schema() -> TemplateSchemaEvidence {
    TemplateSchemaEvidence {
        id: "queue_delivery".to_owned(),
        kind: "declared_must_call".to_owned(),
        template_schema: "lifecycle-template".to_owned(),
        schema_version: 1,
        confidence: "modeled".to_owned(),
        source: "built_in".to_owned(),
        lifecycle_owner: "Delivery".to_owned(),
        cancel_policy: "terminal_action_or_requeue".to_owned(),
        registry_source: "built_in".to_owned(),
        source_span: span(),
    }
}

fn verify(
    certificate: &ProofCertificate,
) -> Result<kobo_proof::VerificationReport, VerificationError> {
    verify_certificate(
        certificate,
        &VerificationContext {
            source: SOURCE.to_owned(),
            source_map: None,
        },
    )
}

fn rehash(certificate: &mut ProofCertificate) {
    certificate.certificate_material_hash =
        certificate_material_hash(certificate).expect("certificate hash should serialize");
}

fn rehash_core(certificate: &mut ProofCertificate) {
    certificate.core.hash = core_material_hash(
        &certificate.core.version,
        &certificate.core.cfg_nodes,
        &certificate.core.cfg_edges,
        &certificate.core.loop_facts,
        &certificate.core.loop_exit_facts,
        &certificate.core.async_model,
    )
    .expect("core hash should serialize");
}

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
