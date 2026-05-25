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

    for required_id in required_obligation_rule_ids() {
        assert!(
            sample.contains(&format!("RuleId.{required_id}")),
            "sample trace must exercise required rule `{required_id}`"
        );
        assert!(
            known_rule_ids.contains(required_id),
            "rule catalog must know sample rule `{required_id}`"
        );
    }
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
        certificate
            .core_obligation_trace
            .iter()
            .any(|event| event.kind == TraceEventKind::Return),
        "sample fixture must include a modeled return trace event"
    );
    assert!(
        certificate
            .core_obligation_trace
            .iter()
            .any(|event| event.kind == TraceEventKind::Panic),
        "sample fixture must include a cancel-or-panic trace event"
    );
    assert!(
        certificate
            .core_obligation_trace
            .iter()
            .any(|event| event.kind == TraceEventKind::OpaqueBoundary),
        "sample fixture must include an opaque boundary trace event"
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

fn sample_trace_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/SampleTrace.lean"))
        .expect("Lean sample trace mirror should exist")
}

fn core_model_source() -> String {
    std::fs::read_to_string(repo_root().join("proof/lean/KoboCore.lean"))
        .expect("Lean core model should exist")
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
