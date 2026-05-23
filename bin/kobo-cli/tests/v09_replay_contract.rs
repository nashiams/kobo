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
        ["backend_version", "backend"].as_slice(),
        ["backend_version", "adapter_source"].as_slice(),
        ["backend_version", "adapter_version"].as_slice(),
        ["backend_replay"].as_slice(),
        ["backend_replay_token"].as_slice(),
        ["backend_controls", "backend"].as_slice(),
        ["backend_controls", "scheduler"].as_slice(),
        ["backend_controls", "max_branches"].as_slice(),
        ["backend_controls", "backend_native"].as_slice(),
        ["backend_controls", "replay_token"].as_slice(),
        ["backend_controls", "checkpoint_replay"].as_slice(),
        ["checkpoint_replay", "enabled"].as_slice(),
        ["sim_profile"].as_slice(),
        ["sim_config", "default_profile"].as_slice(),
        ["sim_config", "show_backend_choices"].as_slice(),
        ["sim_config", "profiles"].as_slice(),
        ["sim_config", "backends"].as_slice(),
        ["replay_guarantee"].as_slice(),
        ["expanded_policy", "ownership"].as_slice(),
        ["expanded_policy", "liveness"].as_slice(),
        ["expanded_policy", "replay"].as_slice(),
        ["expanded_policy", "boundaries"].as_slice(),
        ["expanded_policy", "errors"].as_slice(),
        ["modeled_boundaries"].as_slice(),
        ["opaque_boundaries"].as_slice(),
        ["boundary_assumptions"].as_slice(),
        ["obligations"].as_slice(),
        ["boundary_decisions"].as_slice(),
        ["failure", "code"].as_slice(),
        ["failure", "primary_span"].as_slice(),
        ["failure", "related_spans"].as_slice(),
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
    assert_eq!(json["backend"], "generated-rust-process");
    assert_eq!(
        json["backend_controls"]["backend"],
        "generated-rust-process"
    );
    assert_eq!(json["reserved_backend_fit"][0]["backend"], "shuttle");
    assert_eq!(json["reserved_backend_fit"][0]["status"], "reserved");
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
    assert!(
        json["failure"]["related_spans"]
            .as_array()
            .is_some_and(|spans| !spans.is_empty()),
        "liveness witness should include related source spans for the obligation"
    );
}

#[test]
fn replay_rejects_v1_witness_missing_sim_backend_schema() {
    let project = TestProject::new("replay-missing-sim-schema");
    let witnesses = emit_witness(&project);
    assert!(!witnesses.is_empty(), "witness should exist before replay");
    let witness = &witnesses[0];
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(witness).unwrap()).unwrap();
    json.as_object_mut().unwrap().remove("sim_config");
    json.as_object_mut().unwrap().remove("backend_controls");
    std::fs::write(witness, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let output = run_kobo(&[s("replay"), path_arg(witness)], &project.root);

    assert_failure(&output, "replay must reject missing sim/backend schema");
    let text = output.combined();
    assert_contains(
        &text,
        "missing backend_controls",
        "failure should name the first missing stable backend schema field",
    );
}

#[test]
fn backend_native_replay_rejects_non_native_witness() {
    let project = TestProject::new("replay-backend-native-non-native");
    let witnesses = emit_witness(&project);
    assert!(!witnesses.is_empty(), "witness should exist before replay");

    let output = run_kobo(
        &[s("replay"), path_arg(&witnesses[0]), s("--backend-native")],
        &project.root,
    );

    assert_failure(
        &output,
        "backend-native replay must reject non-native witnesses",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "unsupported backend option",
        "failure should name unsupported backend-native replay",
    );
    assert_contains(
        &text,
        "exact Loom",
        "failure should explain the required native replay witness",
    );
}

#[test]
fn backend_native_replay_validates_recorded_backend_controls() {
    let project = TestProject::new("replay-backend-native-controls");
    let file = project.main_file(
        r#"#[kobo::scenario(profile = "sync")]
fn replay_exact_sync() {
    let value = 1;
    let _copy = value;
}
"#,
    );
    let sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--profile"),
            s("sync"),
            s("--backend"),
            s("loom"),
            s("--scheduler"),
            s("exhaustive"),
            s("--backend-native"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &sim,
        "exact Loom scenario should emit a backend-native witness",
    );
    let witnesses = project.find_files_with_ext("kwit");
    assert!(!witnesses.is_empty(), "witness should exist before replay");
    let witness = &witnesses[0];
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(witness).unwrap()).unwrap();
    assert_eq!(json["backend_replay_evidence"]["backend"], "loom");
    assert_eq!(
        json["backend_replay_evidence"]["source"],
        "loom-generated-harness"
    );
    let native_replay_id = json["backend_replay_evidence"]["native_replay_id"]
        .as_str()
        .expect("backend-native witness should carry a native replay id");
    assert!(
        native_replay_id.starts_with("loom-native:"),
        "native replay id should be backend-owned, not a bare Kobo hash: {json}"
    );
    assert_eq!(
        json["backend_replay"], native_replay_id,
        "backend_replay should copy the native backend replay id"
    );
    assert_ne!(
        json["backend_replay_evidence"]["native_replay_id"],
        json["backend_replay_evidence"]["kobo_verification_hash"],
        "native replay id should stay separate from Kobo verification hashes"
    );
    json["backend_controls"]["scheduler"] = serde_json::json!("pct");
    std::fs::write(witness, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let output = run_kobo(
        &[s("replay"), path_arg(witness), s("--backend-native")],
        &project.root,
    );

    assert_failure(
        &output,
        "backend-native replay must reject mutated backend controls",
    );
    let text = output.combined();
    assert_contains(
        &text,
        "backend_controls.scheduler",
        "failure should name the mutated scheduler control",
    );
    assert_contains(
        &text,
        "scenario debt",
        "failure should keep unsupported native controls explicit",
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
fn exact_replay_fails_when_harness_agreement_is_removed() {
    let project = TestProject::new("replay-missing-harness-agreement");
    let file = project.copy_fixture("replay/transaction_leaks.kobo", "src/transaction.kobo");
    let sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("both"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&sim, "failing sim should emit witness");
    let witness = project.find_files_with_ext("kwit")[0].clone();
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&witness).unwrap()).unwrap();
    json["execution_digest"]["agreement"] = serde_json::Value::String("semantic-only".to_owned());
    std::fs::write(&witness, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let replay = run_kobo(
        &[s("replay"), path_arg(&witness), s("--error-format=json")],
        &project.root,
    );
    assert_failure(
        &replay,
        "exact replay must reject missing harness agreement",
    );
    assert_contains(
        &replay.combined(),
        "K0117",
        "engine mismatch must be explicit",
    );
}

#[test]
fn replay_detects_harness_trace_divergence_not_only_source_hash_change() {
    let project = TestProject::new("replay-harness-trace-divergence");
    let witnesses = emit_witness(&project);
    assert!(
        !witnesses.is_empty(),
        "witness should exist before mutation"
    );
    let path = &witnesses[0];

    let mut json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).expect("witness should read"))
            .expect("witness should parse");
    json["execution_digest"]["harness_trace_hash"] =
        serde_json::Value::String("different-harness-trace".to_owned());
    fs::write(path, serde_json::to_string_pretty(&json).unwrap()).expect("mutated witness writes");

    let output = run_kobo(
        &[s("replay"), path_arg(path), s("--error-format=json")],
        &project.root,
    );

    assert_failure(&output, "mutated harness trace must fail replay");
    let text = output.combined();
    assert_contains(
        &text,
        "K0104",
        "replay must report trace divergence when the harness trace changes",
    );
    assert_contains(
        &text,
        "harness_trace_hash",
        "divergence must name the changed trace",
    );
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
fn replay_divergence_honors_human_error_format() {
    let project = TestProject::new("replay-divergence-human");
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
        &[s("replay"), path_arg(path), s("--error-format=human")],
        &project.root,
    );

    assert_failure(&output, "mutated witness must fail replay");
    let text = output.combined();
    assert_contains(&text, "K0104", "human replay output must name K0104");
    assert_contains(
        &text,
        "expected",
        "human replay output must include expected context",
    );
    assert_contains(
        &text,
        "observed",
        "human replay output must include observed context",
    );
    assert!(
        !text.trim_start().starts_with('{'),
        "human replay output must not be raw JSON:\n{text}"
    );
}

#[test]
fn uncontrolled_effect_witness_is_not_replayable() {
    let project = TestProject::new("not-replayable-witness");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn uncontrolled_effect() {
    let _ = std::fs::read_to_string("state.txt");
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "uncontrolled effect should emit a witness");
    let witnesses = project.find_files_with_ext("kwit");
    assert!(
        !witnesses.is_empty(),
        "uncontrolled effect failure must emit a witness"
    );
    let witness: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&witnesses[0]).expect("witness should be readable"),
    )
    .expect("witness should parse");
    assert_eq!(
        witness["replay_guarantee"].as_str(),
        Some("not_replayable"),
        "uncontrolled effects must not be labeled exact replay"
    );
    assert_json_has_path(
        &witness,
        &["boundary_assumptions"],
        "uncontrolled witness should record replay assumptions",
    );

    let replay = run_kobo(&[s("replay"), path_arg(&witnesses[0])], &project.root);
    assert_failure(&replay, "not_replayable witness must not replay as exact");
    let text = replay.combined();
    assert_contains(
        &text,
        "not_replayable",
        "replay should report the witness guarantee",
    );
    assert_contains(
        &text,
        "uncontrolled",
        "replay should explain the blocking effect",
    );
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
