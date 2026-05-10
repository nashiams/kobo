mod v09_common;

use std::fs;

use v09_common::{
    assert_contains, assert_failure, assert_json_has_path, assert_success, path_arg, run_kobo, s,
    unique_symbol, TestProject,
};

fn emit_witness(project: &TestProject) -> Vec<std::path::PathBuf> {
    let file = project.copy_fixture("replay/transaction_leaks.kobo", "src/transaction.kobo");
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--profile"),
            s("checked"),
            s("--seed"),
            s("9"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "failing scenario should emit a witness");
    project.find_files_with_ext("kwit")
}

#[test]
fn kwit_emitted_for_liveness_failure_has_required_schema() {
    let project = TestProject::new("kwit-schema");
    let witnesses = emit_witness(&project);
    assert!(
        !witnesses.is_empty(),
        "sim failure must emit at least one .kwit witness"
    );

    let witness = fs::read_to_string(&witnesses[0]).expect("witness should be readable");
    let json: serde_json::Value =
        serde_json::from_str(&witness).expect(".kwit should be structured JSON in v0.9");
    for path in [
        ["schema_version"].as_slice(),
        ["kobo_version"].as_slice(),
        ["target"].as_slice(),
        ["guarantee_profile"].as_slice(),
        ["seed"].as_slice(),
        ["backend_profile"].as_slice(),
        ["backend"].as_slice(),
        ["backend_replay"].as_slice(),
        ["replay_guarantee"].as_slice(),
        ["modeled_boundaries"].as_slice(),
        ["opaque_boundaries"].as_slice(),
        ["obligations"].as_slice(),
        ["boundary_decisions"].as_slice(),
        ["failure", "code"].as_slice(),
        ["failure", "primary_span"].as_slice(),
        ["events"].as_slice(),
    ] {
        assert_json_has_path(&json, path, ".kwit witness required field");
    }
    assert_eq!(
        json["kobo_version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "witness must record the real Kobo package version"
    );
    assert_ne!(
        json["kobo_version"].as_str(),
        Some("test"),
        "witness version must not be placeholder text"
    );
    assert_eq!(json["failure"]["code"], "K0100");
    assert_eq!(json["backend"], "shuttle");
    assert_contains(
        &witness,
        "Transaction",
        "witness obligations must record the concrete must_call type",
    );
    assert_contains(
        &witness,
        "commit",
        "witness obligations must record discharge alternatives",
    );
}

#[test]
fn kwit_schema_records_dynamic_target_and_seed() {
    let project = TestProject::new("kwit-dynamic-target");
    let scenario = unique_symbol("transaction_leaks");
    let seed = u64::from(std::process::id()) + 109;
    let file = project.copy_fixture_template(
        "replay/dynamic_transaction.template.kobo",
        "src/dynamic_transaction.kobo",
        &[("__SCENARIO__", &scenario)],
    );
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--profile"),
            s("checked"),
            s("--seed"),
            seed.to_string(),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "dynamic liveness failure should emit witness");
    let witnesses = project.find_files_with_ext("kwit");
    assert!(
        !witnesses.is_empty(),
        "dynamic failure must create .kwit witness"
    );
    let witness = fs::read_to_string(&witnesses[0]).expect("witness should be readable");
    let json: serde_json::Value =
        serde_json::from_str(&witness).expect(".kwit should be structured JSON");
    assert_contains(
        json["target"].as_str().unwrap_or_default(),
        &scenario,
        "witness target must use actual scenario symbol",
    );
    assert_eq!(
        json["seed"].as_u64(),
        Some(seed),
        "witness seed must come from CLI seed, not canned output"
    );
}

#[test]
fn replay_exact_kwit_succeeds_and_reports_same_failure() {
    let project = TestProject::new("replay-exact");
    let witnesses = emit_witness(&project);
    assert!(!witnesses.is_empty(), "witness should exist before replay");

    let output = run_kobo(
        &[
            s("replay"),
            path_arg(&witnesses[0]),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_success(&output, "exact replay should succeed");
    let text = output.combined();
    assert_contains(&text, "exact", "replay must state exactness");
    assert_contains(&text, "K0100", "replay must report original failure code");
}

#[test]
fn replay_divergence_emits_k0104_with_expected_and_observed_events() {
    let project = TestProject::new("replay-divergence");
    let witnesses = emit_witness(&project);
    assert!(
        !witnesses.is_empty(),
        "witness should exist before mutation"
    );
    let path = &witnesses[0];

    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).expect("witness should read"))
            .expect("witness should parse");
    json["events"][0]["kind"] = serde_json::Value::String("mutated-event".to_owned());
    fs::write(path, serde_json::to_string_pretty(&json).unwrap()).expect("mutated witness writes");

    let output = run_kobo(
        &[s("replay"), path_arg(path), s("--error-format=json")],
        &project.root,
    );

    assert_failure(&output, "mutated witness must fail replay");
    let text = output.combined();
    assert_contains(&text, "K0104", "divergence must emit K0104");
    assert_contains(&text, "expected", "K0104 must include expected event");
    assert_contains(&text, "observed", "K0104 must include observed event");
}

#[test]
fn replay_event_payload_divergence_emits_k0104() {
    let project = TestProject::new("replay-payload-divergence");
    let witnesses = emit_witness(&project);
    assert!(
        !witnesses.is_empty(),
        "witness should exist before mutation"
    );
    let path = &witnesses[0];

    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).expect("witness should read"))
            .expect("witness should parse");
    json["events"][0]["label"] = serde_json::Value::String("other_token".to_owned());
    fs::write(path, serde_json::to_string_pretty(&json).unwrap()).expect("mutated witness writes");

    let output = run_kobo(
        &[s("replay"), path_arg(path), s("--error-format=json")],
        &project.root,
    );

    assert_failure(&output, "mutated event payload must fail replay");
    let text = output.combined();
    assert_contains(&text, "K0104", "payload divergence must emit K0104");
    assert_contains(
        &text,
        "expected",
        "K0104 must include expected event payload",
    );
    assert_contains(
        &text,
        "observed",
        "K0104 must include observed event payload",
    );
}

#[test]
fn source_mismatch_is_reported_before_exact_replay() {
    let project = TestProject::new("replay-source-mismatch");
    let witnesses = emit_witness(&project);
    assert!(
        !witnesses.is_empty(),
        "witness should exist before source edit"
    );
    project.write(
        "src/transaction.kobo",
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {
    id: u64,
}

#[kobo::scenario(profile = "async")]
fn transaction_leaks() {
    let tx = Transaction { id: 11 };
    tx.commit();
}
"#,
    );

    let output = run_kobo(
        &[
            s("replay"),
            path_arg(&witnesses[0]),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(&output, "edited source must not replay as exact");
    let text = output.combined();
    assert_contains(&text, "source", "source mismatch should be named");
    assert_contains(&text, "mismatch", "source mismatch should be explicit");
}

#[test]
fn partial_replay_is_labeled_partial_not_exact() {
    let project = TestProject::new("partial-replay");
    let witness = project.copy_fixture("replay/partial.kwit", ".kobo/witnesses/partial.kwit");

    let output = run_kobo(&[s("replay"), path_arg(&witness)], &project.root);
    assert_failure(&output, "partial replay must not claim exact success");
    let text = output.combined();
    assert_contains(&text, "partial", "partial witness must be labeled partial");
    assert_contains(&text, "reqwest::Client", "opaque boundary must be reported");
}
