mod v09_common;

use std::fs;

use v09_common::{
    assert_contains, assert_failure, assert_json_has_path, assert_not_contains, assert_success,
    first_json, one_based_line_of, path_arg, run_kobo, s, unique_symbol, TestProject,
};

#[test]
fn sim_quick_tracks_obligation_discharge_through_helper_call() {
    let project = TestProject::new("prod-depth-helper-discharge");
    let type_name = unique_symbol("DeliveryToken");
    let helper_name = unique_symbol("finish_delivery");
    let scenario_name = unique_symbol("scenario_delivery");
    let source = format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct {type_name} {{}}

fn {helper_name}(delivery: {type_name}) {{
    delivery.ack();
}}

#[kobo::scenario(profile = "async")]
fn {scenario_name}() {{
    let delivery = {type_name} {{}};
    {helper_name}(delivery);
}}
"#
    );
    let file = project.main_file(&source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            path_arg(&file),
            s("--target"),
            scenario_name,
            s("--seed"),
            s("17"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "helper-mediated discharge must be semantically resolved",
    );
    assert_not_contains(
        &output.combined(),
        "K0100",
        "resolved helper call must not leak liveness debt",
    );
}

#[test]
fn sim_quick_does_not_report_raw_nondeterminism_in_unreachable_branch() {
    let project = TestProject::new("prod-depth-dead-branch");
    let chooser = unique_symbol("always_false");
    let scenario = unique_symbol("scenario_dead_branch");
    let source = format!(
        r#"
fn {chooser}() -> bool {{
    false
}}

#[kobo::scenario(profile = "sync")]
fn {scenario}() {{
    if {chooser}() {{
        let _ignored = std::time::SystemTime::now();
    }}
    let _safe = ward.time.now();
}}
"#
    );
    let file = project.main_file(&source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            path_arg(&file),
            s("--target"),
            scenario,
            s("--seed"),
            s("3"),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "dead branch raw nondeterminism must not poison replay path",
    );
    assert_not_contains(
        &output.combined(),
        "K0102",
        "unreachable raw nondeterminism is not on the replay path",
    );
}

#[test]
fn typed_error_policy_ignores_question_marks_in_literals_comments_and_chars() {
    let project = TestProject::new("prod-depth-error-policy-question-literals");
    project.write(
        "Kobo.toml",
        r#"
[guarantees]
errors = "typed"
"#,
    );
    let source = r#"
fn main() -> Result<(), std::io::Error> {
    println!("literal ? must not be rewritten");
    let ch = '?';
    // comment ? must not be rewritten
    Ok(())
}
"#;
    let file = project.main_file(source);
    let output = run_kobo(
        &[
            s("build"),
            s("--profile"),
            s("checked"),
            s("--emit-rust"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "typed error policy must not rewrite non-operator question marks",
    );
    let generated = project.read("src/main.rs");
    assert_contains(
        &generated,
        "literal ? must not be rewritten",
        "string literal must be preserved",
    );
    assert_contains(&generated, "let ch = '?'", "char literal must be preserved");
    assert_not_contains(
        &generated,
        "literal .map_err",
        "string literal question mark must not be rewritten as an error operator",
    );
    assert_not_contains(
        &generated,
        "comment .map_err",
        "comment question mark must not be rewritten as an error operator",
    );
}

#[test]
fn kwit_exact_replay_requires_execution_digest() {
    let project = TestProject::new("prod-depth-digest");
    let scenario = unique_symbol("leaks_delivery");
    let source = format!(
        r#"
#[kobo::must_call(ack | nack)]
struct Delivery {{}}

#[kobo::scenario(profile = "async")]
fn {scenario}() {{
    let delivery = Delivery {{}};
}}
"#
    );
    let file = project.main_file(&source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--profile"),
            s("checked"),
            s("--seed"),
            s("19"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            scenario,
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "failing exact scenario should emit a witness");
    let witnesses = project.find_files_with_ext("kwit");
    assert!(
        !witnesses.is_empty(),
        "failing exact scenario must emit a .kwit witness"
    );
    let witness_path = &witnesses[0];
    let original_text = fs::read_to_string(witness_path).expect("witness should be readable");
    let original: serde_json::Value =
        serde_json::from_str(&original_text).expect("witness should be JSON");
    assert_eq!(original["replay_guarantee"].as_str(), Some("exact"));
    for path in [
        ["execution_digest", "engine"].as_slice(),
        ["execution_digest", "model_schema"].as_slice(),
        ["execution_digest", "schema_version"].as_slice(),
        ["execution_digest", "scenario_ir_hash"].as_slice(),
        ["execution_digest", "operation_count"].as_slice(),
        ["execution_digest", "event_hash"].as_slice(),
    ] {
        assert_json_has_path(&original, path, "exact witness must carry execution digest");
    }
    assert!(
        !original["execution_digest"]["model_schema"]
            .as_str()
            .unwrap_or_default()
            .contains("v0."),
        "public execution digest schema must not expose roadmap-stage wording: {original}"
    );
    assert_eq!(
        original["execution_digest"]["engine"].as_str(),
        Some("semantic-sim"),
        "exact replay must be tied to the semantic executor"
    );
    assert!(
        original["execution_digest"]["operation_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "digest must record a non-empty semantic operation graph"
    );

    let mut message_mutation = original.clone();
    message_mutation["failure"]["message"] =
        serde_json::Value::String("pretty text changed after witness emission".to_owned());
    fs::write(
        witness_path,
        serde_json::to_string_pretty(&message_mutation).unwrap(),
    )
    .expect("mutated witness should write");
    let replay_message_mutation = run_kobo(
        &[
            s("replay"),
            path_arg(witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_success(
        &replay_message_mutation,
        "exact replay must compare structured failure data, not pretty message text",
    );

    let mut digest_mutation = original;
    digest_mutation["execution_digest"]["event_hash"] =
        serde_json::Value::String("mutated-event-hash".to_owned());
    fs::write(
        witness_path,
        serde_json::to_string_pretty(&digest_mutation).unwrap(),
    )
    .expect("mutated witness should write");
    let replay_digest_mutation = run_kobo(
        &[
            s("replay"),
            path_arg(witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(
        &replay_digest_mutation,
        "mutated execution digest must fail exact replay",
    );
    assert_contains(
        &replay_digest_mutation.combined(),
        "K0104",
        "digest divergence must emit K0104",
    );
}

#[test]
fn failure_hooks_change_reachable_outcome_at_effect_site() {
    let project = TestProject::new("prod-depth-failure-hooks");
    let source = r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn cancel_delivery() {
    let delivery = Delivery {};
    ward.task();
    delivery.ack();
}
"#;
    let file = project.main_file(source);

    let baseline = run_kobo(
        &[s("test"), s("--sim"), s("quick"), path_arg(&file)],
        &project.root,
    );
    assert_success(
        &baseline,
        "baseline modeled path should discharge obligation",
    );

    let cancel = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--inject"),
            s("cancel"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &cancel,
        "cancel injection should terminate before the later discharge",
    );
    assert_contains(
        &cancel.combined(),
        "K0100",
        "cancel must cause liveness failure",
    );

    let crash = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--inject"),
            s("crash"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&crash, "crash injection should stop the modeled effect");
    assert_contains(
        &crash.combined(),
        "K0103",
        "crash must cause replay failure",
    );

    let preempt_time_jump = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--inject"),
            s("preempt,time-jump"),
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &preempt_time_jump,
        "preempt and time-jump should alter events without losing obligation",
    );
    let text = preempt_time_jump.combined();
    assert_contains(
        &text,
        "preempt",
        "preempt hook must be present in event stream",
    );
    assert_contains(
        &text,
        "time-jump",
        "time-jump hook must be present in event stream",
    );
}

#[test]
fn k010x_diagnostics_use_semantic_user_spans() {
    let project = TestProject::new("prod-depth-spans");

    let liveness_source = r#"
#[kobo::must_call(ack | nack)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn leaks_delivery() {
    let delivery = Delivery {};
}
"#;
    let liveness_file = project.write("src/liveness.kobo", liveness_source);
    let liveness_line = one_based_line_of(liveness_source, "let delivery");
    let liveness = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&liveness_file),
        ],
        &project.root,
    );
    assert_failure(&liveness, "liveness scenario should fail");
    let liveness_json = first_json(&liveness, "liveness diagnostic must be JSON");
    assert_eq!(
        liveness_json["line"].as_u64(),
        Some(liveness_line as u64),
        "K0100 primary line must point at the semantic obligation creation"
    );

    let raw_source = r#"
#[kobo::scenario(profile = "sync")]
fn raw_clock() {
    let now = std::time::SystemTime::now();
    println!("{:?}", now);
}
"#;
    let raw_file = project.write("src/raw.kobo", raw_source);
    let raw_line = one_based_line_of(raw_source, "SystemTime::now");
    let raw = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&raw_file),
        ],
        &project.root,
    );
    assert_failure(&raw, "raw nondeterminism should fail");
    let raw_json = first_json(&raw, "raw nondeterminism diagnostic must be JSON");
    assert_eq!(
        raw_json["line"].as_u64(),
        Some(raw_line as u64),
        "K0102 primary line must point at the raw nondeterminism expression"
    );

    let boundary_source = r#"
#[kobo::scenario(profile = "async")]
fn external_boundary() {
    let _ = reqwest::Client::new();
}
"#;
    let boundary_file = project.write("src/boundary.kobo", boundary_source);
    let boundary_line = one_based_line_of(boundary_source, "Client::new");
    let boundary = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&boundary_file),
        ],
        &project.root,
    );
    assert_failure(&boundary, "external boundary should fail");
    let boundary_json = first_json(&boundary, "boundary diagnostic must be JSON");
    assert_eq!(
        boundary_json["line"].as_u64(),
        Some(boundary_line as u64),
        "K0107 primary line must point at the external boundary call"
    );
}
