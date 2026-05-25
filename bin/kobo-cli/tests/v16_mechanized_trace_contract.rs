use kobo_proof::{
    load_obligation_rule_catalog, parse_certificate_json, required_obligation_rule_ids,
    verify_certificate, TraceEventKind, VerificationContext,
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
            source_map: None,
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

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kobo-cli manifest should live under bin/kobo-cli")
        .to_path_buf()
}
