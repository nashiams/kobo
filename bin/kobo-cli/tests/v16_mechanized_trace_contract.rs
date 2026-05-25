use kobo_proof::{load_obligation_rule_catalog, required_obligation_rule_ids};

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
}

#[test]
fn stale_template_version_is_not_the_sample_assumption() {
    let sample = sample_trace_source();

    assert!(
        !sample.contains("templateVersion := \"stale\""),
        "sample trace must not satisfy the contract with a stale template version"
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

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kobo-cli manifest should live under bin/kobo-cli")
        .to_path_buf()
}
