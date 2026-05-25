use std::collections::BTreeSet;

use kobo_proof::{
    load_obligation_rule_catalog, parse_certificate_json, parse_obligation_rule_catalog,
    required_obligation_rule_ids, validate_obligation_rule_catalog, ObligationEventKind,
    ObligationState,
};

#[test]
fn rule_list_contains_the_eight_required_stable_ids() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let observed = catalog
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();

    for required_id in required_obligation_rule_ids() {
        assert!(
            observed.contains(required_id),
            "missing required v0.16 obligation rule id `{required_id}`"
        );
    }

    validate_obligation_rule_catalog(&catalog).expect("default rule catalog should validate");
}

#[test]
fn missing_required_rule_ids_are_rejected() {
    for required_id in required_obligation_rule_ids() {
        let source = default_rule_catalog_source().replace(
            &format!("id = \"{required_id}\""),
            "id = \"mutated_missing_required_rule\"",
        );
        let catalog = parse_obligation_rule_catalog(&source).expect("mutated catalog should parse");
        let error = validate_obligation_rule_catalog(&catalog)
            .expect_err("missing required rule id must fail validation");
        assert!(
            error.to_string().contains(required_id),
            "error should name missing rule `{required_id}`: {error}"
        );
    }
}

#[test]
fn transition_drift_from_rust_expectations_is_rejected() {
    let source = default_rule_catalog_source().replace(
        "id = \"create\"\nname = \"Create\"\ninput_states = []\noutput_states = [\"owned\"]",
        "id = \"create\"\nname = \"Create\"\ninput_states = []\noutput_states = [\"resolved\"]",
    );
    let catalog = parse_obligation_rule_catalog(&source).expect("mutated catalog should parse");
    let error = validate_obligation_rule_catalog(&catalog)
        .expect_err("rule transition drift must fail validation");

    assert!(
        error.to_string().contains("create"),
        "error should identify the drifted create rule: {error}"
    );
}

#[test]
fn missing_lean_theorem_names_are_rejected() {
    let source = default_rule_catalog_source().replace(
        "lean_theorem = \"preservation_create\"",
        "lean_theorem = \"\"",
    );
    let catalog = parse_obligation_rule_catalog(&source).expect("mutated catalog should parse");
    let error = validate_obligation_rule_catalog(&catalog)
        .expect_err("missing Lean theorem name must fail validation");

    assert!(
        error.to_string().contains("create"),
        "error should identify the rule missing a Lean theorem: {error}"
    );
}

#[test]
fn rule_entries_name_real_rust_tests_and_lean_declarations() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let proof_test_sources = proof_test_sources();
    let lean_sources = lean_sources();

    for rule in catalog.rules {
        assert!(
            proof_test_sources.contains(&format!("fn {}", rule.rust_test)),
            "rule `{}` names missing Rust verifier test `{}`",
            rule.id,
            rule.rust_test
        );
        assert!(
            lean_sources.contains(&format!("| {}", rule.lean_rule))
                || lean_sources.contains(&format!("theorem {}", rule.lean_rule)),
            "rule `{}` names missing Lean rule `{}`",
            rule.id,
            rule.lean_rule
        );
        assert!(
            lean_sources.contains(&format!("theorem {}", rule.lean_theorem)),
            "rule `{}` names missing Lean theorem `{}`",
            rule.id,
            rule.lean_theorem
        );
    }
}

#[test]
fn rule_projection_covers_shipped_rust_obligation_events() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let projected_events = catalog
        .rules
        .iter()
        .flat_map(|rule| rule.rust_event_kinds.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();

    for rust_event_kind in [
        "create",
        "discharge",
        "transfer",
        "move",
        "branch_unresolved",
        "escape",
    ] {
        assert!(
            projected_events.contains(rust_event_kind),
            "v0.16 rule projection must account for shipped Rust event `{rust_event_kind}`"
        );
    }
}

#[test]
fn rule_transitions_match_sample_certificate_obligation_events() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let source = sample_kproof_source();
    let certificate = parse_certificate_json(&source).expect("sample .kproof should parse");

    for event in certificate.obligation_events {
        let Some(rule_id) = rule_id_for_obligation_event(&event.kind) else {
            continue;
        };
        let rule = catalog
            .rules
            .iter()
            .find(|rule| rule.id == rule_id)
            .unwrap_or_else(|| panic!("catalog should contain event rule `{rule_id}`"));

        let event_input_states = state_names(&event.state_before);
        let catalog_input_states = rule
            .input_states
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let event_output_states = state_names(&event.state_after);
        let catalog_output_states = rule
            .output_states
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();

        assert!(
            event_input_states.is_subset(&catalog_input_states),
            "rule `{rule_id}` input states must admit sample certificate event `{}`: event={event_input_states:?} catalog={catalog_input_states:?}",
            event.id
        );
        assert!(
            event_output_states.is_subset(&catalog_output_states),
            "rule `{rule_id}` output states must admit sample certificate event `{}`: event={event_output_states:?} catalog={catalog_output_states:?}",
            event.id
        );
    }
}

#[test]
fn rule_entries_name_existing_rust_verifier_hooks() {
    let catalog = load_obligation_rule_catalog().expect("rule catalog should parse");
    let verifier_sources = proof_verifier_sources();

    for rule in catalog.rules {
        let marker = rust_verifier_marker(&rule.rust_verifier);
        assert!(
            verifier_sources.contains(marker),
            "rule `{}` names missing Rust verifier marker `{}` from `{}`",
            rule.id,
            marker,
            rule.rust_verifier
        );
    }
}

fn default_rule_catalog_source() -> String {
    std::fs::read_to_string(
        repo_root().join("crates/compiler/kobo-proof/rules/obligation_rules.toml"),
    )
    .expect("default obligation rule catalog should exist")
}

fn proof_test_sources() -> String {
    let test_dir = repo_root().join("crates/compiler/kobo-proof/tests");
    std::fs::read_dir(test_dir)
        .expect("kobo-proof test dir should exist")
        .map(|entry| {
            let path = entry.expect("test entry should read").path();
            std::fs::read_to_string(path).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn lean_sources() -> String {
    std::fs::read_dir(repo_root().join("proof/lean"))
        .expect("Lean proof dir should exist")
        .map(|entry| {
            let path = entry.expect("Lean entry should read").path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("lean") {
                return String::new();
            }
            std::fs::read_to_string(path).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn proof_verifier_sources() -> String {
    let source_dir = repo_root().join("crates/compiler/kobo-proof/src");
    std::fs::read_dir(source_dir)
        .expect("kobo-proof source dir should exist")
        .map(|entry| {
            let path = entry.expect("source entry should read").path();
            if path.file_name().and_then(|name| name.to_str()) == Some("rule_sync.rs") {
                return String::new();
            }
            std::fs::read_to_string(path).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sample_kproof_source() -> String {
    std::fs::read_to_string(
        repo_root().join("crates/compiler/kobo-proof/fixtures/v16_sample.kproof"),
    )
    .expect("sample .kproof fixture should exist")
}

fn rule_id_for_obligation_event(kind: &ObligationEventKind) -> Option<&'static str> {
    match kind {
        ObligationEventKind::Create => Some("create"),
        ObligationEventKind::Discharge => Some("discharge"),
        ObligationEventKind::Transfer | ObligationEventKind::Move => Some("transfer"),
        ObligationEventKind::BranchUnresolved => Some("split"),
        ObligationEventKind::Escape => Some("opaque"),
        ObligationEventKind::UnsupportedContainer | ObligationEventKind::Call => None,
    }
}

fn state_names(states: &[ObligationState]) -> BTreeSet<&str> {
    states.iter().map(|state| state.state.as_str()).collect()
}

fn rust_verifier_marker(verifier: &str) -> &str {
    match verifier {
        "apply_obligation_event::Create" => "ObligationEventKind::Create",
        "apply_obligation_event::Transfer" => "ObligationEventKind::Transfer",
        "apply_obligation_event::Discharge" => "ObligationEventKind::Discharge",
        "apply_obligation_event::BranchUnresolved" => "ObligationEventKind::BranchUnresolved",
        "reject_unresolved_exit::return" | "reject_unresolved_exit::panic" => {
            "reject_unresolved_exit"
        }
        "verify_boundary_policies::Opaque" => "BoundaryPolicy::Opaque",
        other => other,
    }
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kobo-cli manifest should live under bin/kobo-cli")
        .to_path_buf()
}
