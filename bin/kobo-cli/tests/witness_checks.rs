mod cli_test_support;

use std::fs;

use cli_test_support::{
    assert_contains, assert_failure, assert_json_has_path, assert_success, path_arg, run_kobo, s,
    TestProject,
};
use serde_json::Value;

fn emit_runtime_witness(project: &TestProject) -> std::path::PathBuf {
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn async_gateway() {
    let reply = ReplyToken {};
    ward.task();
    let _lost = reply;
}
"#,
    );
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("11"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "async gateway should emit unresolved reply witness",
    );
    project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist")
}

#[test]
fn kwit_contains_runtime_required_fields_and_shrink_metadata() {
    let project = TestProject::new("runtime-witness-fields");
    let witness_path = emit_runtime_witness(&project);
    let witness_text = fs::read_to_string(&witness_path).expect("witness should read");
    let witness: Value = serde_json::from_str(&witness_text).expect("witness should parse");

    for path in [
        ["sim_profile"].as_slice(),
        ["backend_profile"].as_slice(),
        ["backend_version", "backend"].as_slice(),
        ["backend_version", "adapter_version"].as_slice(),
        ["backend_replay_token"].as_slice(),
        ["backend_controls", "replay_token"].as_slice(),
        ["checkpoint_replay", "enabled"].as_slice(),
        ["event_stream"].as_slice(),
        ["boundary_policies"].as_slice(),
        ["obligation_events"].as_slice(),
        ["exactness"].as_slice(),
        ["shrink", "replay_checked"].as_slice(),
        ["coverage", "covered"].as_slice(),
        ["source_spans"].as_slice(),
    ] {
        assert_json_has_path(&witness, path, "runtime witness required field");
    }
    assert_eq!(witness["sim_profile"], "quick");
    assert_contains(
        &witness_text,
        "unresolved-reply",
        "async gateway witness should name unresolved reply failure mode",
    );
}

#[test]
fn unsafe_shrink_metadata_is_rejected_with_k0106() {
    let project = TestProject::new("runtime-unsafe-shrink");
    let witness_path = emit_runtime_witness(&project);
    let mut witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    witness["shrink"]["replay_checked"] = Value::Bool(false);
    witness["exactness"] = Value::String("exact".to_owned());
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(&witness).unwrap(),
    )
    .expect("mutated witness should write");

    let output = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(&output, "unsafe shrink must be rejected before replay");
    assert_contains(
        &output.combined(),
        "K0106",
        "unsafe shrink should emit K0106",
    );
}

#[test]
fn exact_replay_rejects_mutated_function_summaries() {
    let project = TestProject::new("runtime-mutated-function-summaries");
    let witness_path = emit_runtime_witness(&project);
    let mut witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    witness["function_summaries"] = serde_json::json!([{
        "function": "forged",
        "creates": [],
        "transfers": [],
        "discharges": ["reply"],
        "leaks": [],
        "returns": [],
        "escapes": [],
        "suppressed": []
    }]);
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(&witness).unwrap(),
    )
    .expect("mutated witness should write");

    let output = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "exact replay must reject forged function summaries",
    );
    assert_contains(
        &output.combined(),
        "function_summaries",
        "replay divergence should name function_summaries",
    );
}

#[test]
fn exact_replay_rejects_mutated_operation_coverage() {
    let project = TestProject::new("runtime-mutated-operation-coverage");
    let witness_path = emit_runtime_witness(&project);
    let mut witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    witness["operation_coverage"]["modeled"] = serde_json::json!(["forged-coverage"]);
    fs::write(
        &witness_path,
        serde_json::to_string_pretty(&witness).unwrap(),
    )
    .expect("mutated witness should write");

    let output = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );

    assert_failure(
        &output,
        "exact replay must reject forged operation coverage",
    );
    assert_contains(
        &output.combined(),
        "operation_coverage",
        "replay divergence should name operation_coverage",
    );
}

#[test]
fn deep_witness_shrinks_scheduler_events_and_replays_exactly() {
    let project = TestProject::new("runtime-shrunk-witness");
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn async_gateway() {
    let reply = ReplyToken {};
    ward.random.u64();
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("31"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "deep async gateway should emit a witness");

    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    assert!(
        witness["shrink"]["original_event_count"].as_u64()
            > witness["shrink"]["shrunk_event_count"].as_u64(),
        "deep profile should shrink independent scheduler events: {witness}"
    );
    assert!(
        witness["shrink"]["removed_event_ids"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty()),
        "shrink provenance should name removed event ids: {witness}"
    );
    assert_eq!(
        witness["events"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default() as u64,
        witness["shrink"]["shrunk_event_count"].as_u64().unwrap()
    );

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_success(&replay, "shrunk witness should replay exactly");
    assert_contains(
        &replay.combined(),
        r#""replay":"exact""#,
        "replay should keep exactness after shrink",
    );
}

#[test]
fn configured_shrink_off_preserves_deep_event_stream() {
    let project = TestProject::new("runtime-shrink-off-config");
    project.write(
        "Kobo.toml",
        r#"[sim.profile.deep]
shrink = "off"
"#,
    );
    let file = project.main_file(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {}

#[kobo::scenario(profile = "async")]
fn async_gateway() {
    let reply = ReplyToken {};
    ward.random.u64();
    ward.task();
    let _lost = reply;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("deep"),
            s("--seed"),
            s("31"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "deep async gateway should emit a witness");

    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness: Value =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");

    assert_eq!(witness["shrink"]["mode"], "off");
    assert_eq!(
        witness["shrink"]["original_event_count"], witness["shrink"]["shrunk_event_count"],
        "shrink=off must preserve the full deep event stream"
    );
    assert!(
        witness["shrink"]["removed_event_ids"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "shrink=off must not remove scheduler evidence: {witness}"
    );
}

#[test]
fn cancellation_hook_is_recorded_and_replayed_at_distinct_facade_points() {
    let task_first = TestProject::new("runtime-cancel-task-first");
    let (task_path, task_witness) =
        emit_cancel_witness(&task_first, "ward.task();\n    ward.time.now();");
    let time_first = TestProject::new("runtime-cancel-time-first");
    let (time_path, time_witness) =
        emit_cancel_witness(&time_first, "ward.time.now();\n    ward.task();");

    assert_ne!(
        task_witness["event_stream"], time_witness["event_stream"],
        "cancellation at different facade points must not collapse to one canned witness"
    );
    for witness in [&task_witness, &time_witness] {
        let text = serde_json::to_string(witness).expect("witness should serialize");
        assert_contains(
            &text,
            "failure-injection-cancel",
            "witness must record cancellation hook",
        );
        assert_contains(
            &text,
            "unresolved-reply",
            "cancelled async gateway should expose unresolved reply obligation",
        );
        assert_json_has_path(
            witness,
            &["source_spans"],
            "cancel witness needs source spans",
        );
        assert_json_has_path(
            witness,
            &["injections", "hooks"],
            "cancel replay needs hooks",
        );
    }

    let task_replay = run_kobo(
        &[s("replay"), path_arg(&task_path), s("--error-format=json")],
        &task_first.root,
    );
    assert_success(&task_replay, "task cancellation witness should replay");
    let time_replay = run_kobo(
        &[s("replay"), path_arg(&time_path), s("--error-format=json")],
        &time_first.root,
    );
    assert_success(&time_replay, "time cancellation witness should replay");
}

fn emit_cancel_witness(project: &TestProject, hook_order: &str) -> (std::path::PathBuf, Value) {
    let file = project.main_file(&format!(
        r#"
#[kobo::must_call(reply | reject | cancel)]
struct ReplyToken {{}}

#[kobo::scenario(profile = "async")]
fn async_gateway() {{
    let reply = ReplyToken {{}};
    {hook_order}
    let _lost = reply;
}}
"#
    ));
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--seed"),
            s("41"),
            s("--inject"),
            s("cancel"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "cancel injection should emit witness");
    let witness_path = project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist");
    let witness =
        serde_json::from_str(&fs::read_to_string(&witness_path).expect("witness should read"))
            .expect("witness should parse");
    (witness_path, witness)
}
