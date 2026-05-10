mod v09_common;

use std::fs;

use v09_common::{
    assert_contains, assert_failure, assert_json_has_path, assert_success, path_arg, run_kobo, s,
    TestProject,
};

const FAILING_SCENARIO: &str = r#"
#[kobo::must_call(commit | rollback)]
struct Transaction {
    id: u64,
}

#[kobo::scenario(profile = "async")]
fn transaction_leaks() {
    let tx = Transaction { id: 11 };
    let _lost = tx;
}
"#;

fn emit_witness(project: &TestProject) -> Vec<std::path::PathBuf> {
    let file = project.write("src/transaction.kobo", FAILING_SCENARIO);
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
        ["replay_guarantee"].as_slice(),
        ["modeled_boundaries"].as_slice(),
        ["opaque_boundaries"].as_slice(),
        ["failure", "code"].as_slice(),
        ["failure", "primary_span"].as_slice(),
        ["events"].as_slice(),
    ] {
        assert_json_has_path(&json, path, ".kwit witness required field");
    }
    assert_eq!(json["failure"]["code"], "K0100");
}

#[test]
fn replay_exact_kwit_succeeds_and_reports_same_failure() {
    let project = TestProject::new("replay-exact");
    let witnesses = emit_witness(&project);
    assert!(!witnesses.is_empty(), "witness should exist before replay");

    let output = run_kobo(
        &[s("replay"), path_arg(&witnesses[0]), s("--error-format=json")],
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
    assert!(!witnesses.is_empty(), "witness should exist before mutation");
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
fn partial_replay_is_labeled_partial_not_exact() {
    let project = TestProject::new("partial-replay");
    let witness = project.write(
        ".kobo/witnesses/partial.kwit",
        r#"{
  "schema_version": 1,
  "kobo_version": "test",
  "target": "src/http.kobo:call",
  "guarantee_profile": "checked",
  "seed": 1,
  "backend_profile": "async",
  "replay_guarantee": "partial",
  "modeled_boundaries": ["ward.time"],
  "opaque_boundaries": ["reqwest::Client"],
  "failure": {"code": "K0103", "primary_span": "src/http.kobo:1:1"},
  "events": [{"kind": "outside-boundary"}]
}"#,
    );

    let output = run_kobo(&[s("replay"), path_arg(&witness)], &project.root);
    assert_failure(&output, "partial replay must not claim exact success");
    let text = output.combined();
    assert_contains(&text, "partial", "partial witness must be labeled partial");
    assert_contains(&text, "reqwest::Client", "opaque boundary must be reported");
}
