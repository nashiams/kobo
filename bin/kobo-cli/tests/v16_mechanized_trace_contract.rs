use kobo_proof::{
    load_obligation_rule_catalog, parse_certificate_json, required_obligation_rule_ids,
    verify_certificate, TraceEventKind, TranslationValidationStatus, VerificationContext,
};

const SAMPLE_KOBO_SOURCE: &str =
    "fn proof_case() { let delivery = Delivery {}; helper(delivery); delivery.ack(); }";

#[test]
fn sample_trace_uses_only_known_rule_ids() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let known_rule_ids = catalog
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let sample = sample_trace_source();
    let source = sample_kproof_source();
    let certificate = parse_certificate_json(&source).expect("sample .kproof should parse");

    for required_id in required_obligation_rule_ids() {
        assert!(
            known_rule_ids.contains(required_id),
            "rule catalog must know required rule `{required_id}`"
        );
    }
    for rule_id in certificate
        .core_obligation_trace
        .iter()
        .filter_map(core_trace_rule_id)
    {
        assert!(
            sample.contains(&format!("RuleId.{rule_id}")),
            "sample trace must mirror fixture rule `{rule_id}`"
        );
        assert!(
            known_rule_ids.contains(rule_id),
            "rule catalog must know fixture rule `{rule_id}`"
        );
    }
    assert!(sample.contains("RuleId.create"));
    assert!(sample.contains("RuleId.transfer") || sample.contains("RuleId.split"));
    assert!(sample.contains("RuleId.discharge"));
    assert!(sample.contains("RuleId.return"));
    assert!(sample.contains("RuleId.cancel") || sample.contains("RuleId.panic"));
    assert!(sample.contains("RuleId.opaque"));
}

#[test]
fn sample_trace_records_template_id_version_assumptions() {
    let sample = sample_trace_source();

    assert!(
        sample.contains("templateId := \"queue_delivery\""),
        "sample trace must record a protocol-template assumption id"
    );
    assert!(
        sample.contains("templateVersion := \"0.1\""),
        "sample trace must record the protocol-template assumption version"
    );
    assert!(
        sample.contains("opaqueLedgerRecorded := true"),
        "sample trace must include an explicit opaque ledger assumption"
    );
    for required_field in [
        "rustCertificateFieldPath := \"template_schemas\"",
        "leanAssumptionName := \"queue_delivery_assumption\"",
        "source := TemplateAssumptionSource.builtIn",
        "confidence := TemplateAssumptionConfidence.modeled",
    ] {
        assert!(
            sample.contains(required_field),
            "sample trace must record structured template-assumption field `{required_field}`"
        );
    }
}

#[test]
fn sample_kproof_fixture_is_accepted_by_the_rust_verifier() {
    let source = sample_kproof_source();
    let certificate = parse_certificate_json(&source).expect("sample .kproof should parse");
    let report = verify_certificate(
        &certificate,
        &VerificationContext {
            source: SAMPLE_KOBO_SOURCE.to_owned(),
            source_map: Some(sample_source_map_source()),
        },
    )
    .expect("sample .kproof should verify");

    assert_eq!(
        report.checked_obligation_events, 3,
        "sample fixture should replay create, transfer, and discharge obligation events"
    );
    assert!(
        sample_trace_source().contains(&lean_trace_literal(&certificate)),
        "Lean sample trace must mirror the exact fixture core trace sequence"
    );
}

#[test]
fn sample_kproof_bridge_uses_validated_generated_trace() {
    let source = sample_kproof_source();
    let certificate = parse_certificate_json(&source).expect("sample .kproof should parse");

    assert!(
        !certificate.generated_rust_trace.is_empty(),
        "sample bridge must include generated trace evidence"
    );
    assert_eq!(
        certificate.translation_validation.status,
        TranslationValidationStatus::Validated,
        "sample bridge must not rely on core-only translation validation"
    );
    assert!(
        certificate
            .trace_hashes
            .iter()
            .any(|hash| hash.id == "core_obligation_trace"),
        "sample bridge must hash the Core trace"
    );
    assert!(
        certificate
            .trace_hashes
            .iter()
            .any(|hash| hash.id == "generated_rust_trace"),
        "sample bridge must hash the generated trace"
    );
}

#[test]
fn sample_terminal_trace_events_are_cfg_backed() {
    let source = sample_kproof_source();
    let certificate = parse_certificate_json(&source).expect("sample .kproof should parse");
    let cfg_targets = certificate
        .core
        .cfg_edges
        .iter()
        .map(|edge| edge.to.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    for required_target in ["return", "panic", "opaque_boundary"] {
        assert!(
            cfg_targets.contains(required_target),
            "sample fixture terminal trace must be backed by CFG target `{required_target}`"
        );
    }
    let sample = sample_trace_source();
    assert!(
        sample.contains("ModeledExitStep sampleAfterDischarge ModeledExit.return")
            && sample.contains("ModeledExitStep sampleAfterDischarge ModeledExit.panic")
            && sample.contains("ModeledExitStep sampleAfterDischarge ModeledExit.opaqueBoundary"),
        "Lean sample must model terminal exits as alternatives from the same CFG terminal state"
    );
}

#[test]
fn opaque_ledger_entries_are_cfg_edge_backed() {
    let source = sample_kproof_source();
    let certificate = parse_certificate_json(&source).expect("sample .kproof should parse");
    let opaque_cfg_edges = certificate
        .core
        .cfg_edges
        .iter()
        .filter(|edge| edge.kind == "opaque_boundary" && edge.to == "opaque_boundary")
        .map(|edge| edge.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();

    for ledger in &certificate.opaque_edge_ledger {
        assert!(
            opaque_cfg_edges.contains(ledger.edge_id.as_str()),
            "opaque ledger edge `{}` must name a real opaque CFG edge",
            ledger.edge_id
        );
    }
}

#[test]
fn lean_sample_models_terminal_exits_as_alternatives() {
    let sample = sample_trace_source();

    assert!(
        sample.contains("def sampleObligationTrace")
            && sample.contains("def sampleModeledExitTrace")
            && sample.contains("sample_return_exit_accepted")
            && sample.contains("sample_panic_exit_accepted")
            && sample.contains("sample_opaque_exit_recorded"),
        "Lean sample must split obligation steps from terminal modeled exits"
    );
    assert!(
        !sample.contains("TraceAccepted sampleStart sampleTrace sampleEnd"),
        "Lean sample must not accept terminal exits as one sequential obligation trace"
    );
}

#[test]
fn mechanized_theorems_use_typed_env_and_accounting_evidence() {
    let core = core_model_source();
    let preservation = preservation_source();
    let no_silent_loss = no_silent_loss_source();

    assert!(
        core.contains("typedObligationEnv"),
        "core model must define a typed obligation environment invariant"
    );
    assert!(
        preservation.contains("typedObligationEnv env")
            && preservation.contains("typedObligationEnv next"),
        "preservation theorem must preserve the typed obligation environment"
    );
    assert!(
        no_silent_loss.contains("PermittedAccountingRule")
            && no_silent_loss.contains("unresolved_local_state_change_requires_accounting"),
        "no-silent-loss proof must account for state-changing exits, not only missing keys"
    );
    assert!(
        no_silent_loss.contains("opaque_exit_change_has_ledger_accounting")
            && !no_silent_loss.contains("(_unresolved"),
        "no-silent-loss proof must use unresolved premises and expose opaque ledger accounting"
    );
}

#[test]
fn preservation_and_loss_theorems_use_rule_specific_shapes() {
    let core = core_model_source();
    let rules = rules_model_source();
    let preservation = preservation_source();
    let no_silent_loss = no_silent_loss_source();

    for required in [
        "RuleInputState",
        "RuleOutputState",
        "ruleOutputStateKnown",
        "step_output_state_matches_catalog",
        "preservation_rule_output_matches_catalog",
        "cfgEdgeKind",
        "cfgEdgeTarget",
        "opaqueLedgerBindsCfgEdge",
        "opaque_exit_change_uses_ledger_cfg_edge",
    ] {
        assert!(
            core.contains(required)
                || rules.contains(required)
                || preservation.contains(required)
                || no_silent_loss.contains(required),
            "mechanized proofs must use rule-specific shape `{required}`"
        );
    }
}

#[test]
fn template_and_opaque_evidence_are_semantic_premises() {
    let core = core_model_source();
    let rules = rules_model_source();
    let sample = sample_trace_source();
    let certificate_bridge = certificate_soundness_source();

    for required in [
        "templateAssumptionMatches",
        "templateId = \"queue_delivery\"",
        "obligationKind = \"Delivery\"",
        "statement = \"Delivery obligations are discharged",
        "source = TemplateAssumptionSource.builtIn",
        "confidence = TemplateAssumptionConfidence.modeled",
        "opaqueLedgerRecorded ledger id \"external.queue\"",
        "sample_template_assumption_matches",
        "sample_opaque_ledger_recorded",
    ] {
        assert!(
            core.contains(required)
                || rules.contains(required)
                || sample.contains(required)
                || certificate_bridge.contains(required),
            "mechanized bridge must use semantic evidence premise `{required}`"
        );
    }
}

#[test]
fn cancellation_evidence_is_a_semantic_premise() {
    let core = core_model_source();
    let rules = rules_model_source();

    for required in [
        "structure CancellationEdgeEvidence",
        "structure FutureStateObligationEvidence",
        "cancellationEvidenceRecorded",
        "cancelEvidence : CancellationEdgeEvidence",
        "futureObligation : FutureStateObligationEvidence",
    ] {
        assert!(
            core.contains(required) || rules.contains(required),
            "cancel rule must mechanize evidence premise `{required}`"
        );
    }
}

#[test]
fn stale_template_version_is_structurally_rejected_by_the_sample_model() {
    let sample = format!("{}\n{}", core_model_source(), sample_trace_source());

    assert!(
        sample.contains("def templateAssumptionIsCurrent")
            && sample.contains("templateVersion = \"0.1\""),
        "sample model must structurally reject stale template versions"
    );
}

#[test]
fn mechanized_trace_files_are_release_checked_content() {
    for relative in [
        "proof/lean/KoboCore.lean",
        "proof/lean/ObligationRules.lean",
        "proof/lean/Preservation.lean",
        "proof/lean/NoSilentLoss.lean",
        "proof/lean/SampleTrace.lean",
        "proof/lean/CertificateSoundness.lean",
    ] {
        let source = std::fs::read_to_string(repo_root().join(relative))
            .unwrap_or_else(|error| panic!("{relative} should exist: {error}"));
        assert!(
            source
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count()
                >= 8,
            "{relative} must not be an empty mechanization shell"
        );
        assert!(
            !source.contains("sorry") && !source.contains("admit"),
            "{relative} must not contain unfinished proof stubs"
        );
    }
}

#[test]
fn mechanized_rules_encode_catalog_state_preconditions() {
    let rules = rules_model_source();
    let core = core_model_source();

    for required in [
        "requiresState env id ObligationState.owned",
        "requiresState env id ObligationState.transferred",
        "templateAssumptionIsCurrent assumption",
        "opaqueLedgerRecorded ledger id",
    ] {
        assert!(
            rules.contains(required) || core.contains(required),
            "Lean semantics must encode required premise `{required}`"
        );
    }
}

#[test]
fn no_silent_loss_is_over_modeled_exit_transition() {
    let source = no_silent_loss_source();

    assert!(
        source.contains("ModeledExitStep before exit after")
            && source.contains("before obligation.id = some obligation")
            && source.contains("after obligation.id = none"),
        "no-silent-loss theorem must reason over before/after modeled exit states"
    );
}

#[test]
fn no_silent_loss_rejects_unaccounted_state_change_not_only_missing_keys() {
    let source = no_silent_loss_source();

    assert!(
        source.contains("inductive SilentLossOnModeledExit")
            && source.contains("unaccountedStateChange")
            && source.contains("loss : SilentLossOnModeledExit before after exit obligation"),
        "no-silent-loss theorem must encode unaccounted unresolved-state changes as silent loss"
    );
    assert!(
        !source.contains("(accounting : PermittedAccountingRule rule)"),
        "state-change accounting must be derived from the modeled transition, not assumed"
    );
}

fn sample_trace_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/SampleTrace.lean"))
        .expect("Lean sample trace mirror should exist")
}

fn rules_model_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/ObligationRules.lean"))
        .expect("Lean obligation rules should exist")
}

fn no_silent_loss_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/NoSilentLoss.lean"))
        .expect("Lean no-silent-loss proof should exist")
}

fn preservation_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/Preservation.lean"))
        .expect("Lean preservation proof should exist")
}

fn certificate_soundness_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/CertificateSoundness.lean"))
        .expect("Lean certificate bridge should exist")
}

fn core_model_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/KoboCore.lean"))
        .expect("Lean core model should exist")
}

fn lean_trace_literal(certificate: &kobo_proof::ProofCertificate) -> String {
    let rule_ids = certificate
        .core_obligation_trace
        .iter()
        .filter_map(core_trace_rule_id)
        .map(|rule_id| format!("RuleId.{rule_id}"))
        .collect::<Vec<_>>();
    format!(
        "def sampleTrace : List RuleId :=\n  [{}]",
        rule_ids.join(", ")
    )
}

fn core_trace_rule_id(event: &kobo_proof::CoreTraceEvent) -> Option<&'static str> {
    match event.kind {
        TraceEventKind::Create => Some("create"),
        TraceEventKind::Transfer => Some("transfer"),
        TraceEventKind::Discharge => Some("discharge"),
        TraceEventKind::Return => Some("return"),
        TraceEventKind::Panic => Some("panic"),
        TraceEventKind::OpaqueBoundary => Some("opaque"),
        TraceEventKind::Cancel => Some("cancel"),
        _ => None,
    }
}

fn sample_kproof_source() -> String {
    std::fs::read_to_string(
        repo_root().join("crates/compiler/kobo-proof/fixtures/v16_sample.kproof"),
    )
    .expect("sample .kproof fixture should exist")
}

fn sample_source_map_source() -> String {
    std::fs::read_to_string(
        repo_root().join("crates/compiler/kobo-proof/fixtures/v16_sample.sourcemap.json"),
    )
    .expect("sample source map fixture should exist")
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kobo-cli manifest should live under bin/kobo-cli")
        .to_path_buf()
}
