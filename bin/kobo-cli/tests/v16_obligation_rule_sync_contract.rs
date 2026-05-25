use std::collections::BTreeSet;

use kobo_proof::{
    load_obligation_rule_catalog, parse_obligation_rule_catalog, required_obligation_rule_ids,
    validate_obligation_rule_catalog,
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

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kobo-cli manifest should live under bin/kobo-cli")
        .to_path_buf()
}
