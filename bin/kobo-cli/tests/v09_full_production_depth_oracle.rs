mod v09_common;

use std::fs;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, assert_json_has_path, assert_not_contains, assert_success,
    path_arg, run_kobo, s, unique_symbol, TestProject,
};

fn first_witness(project: &TestProject) -> std::path::PathBuf {
    let witnesses = project.find_files_with_ext("kwit");
    assert!(!witnesses.is_empty(), "expected a .kwit witness");
    witnesses[0].clone()
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("json file should be readable"))
        .expect("file should contain JSON")
}

#[test]
fn exact_replay_requires_compiler_semantics_and_harness_agreement() {
    let project = TestProject::new("full-depth-agreement");
    let token = unique_symbol("DeliveryToken");
    let finish = unique_symbol("finish_delivery");
    let scenario = unique_symbol("scenario_exact");
    let source = format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct {token} {{}}

fn {finish}(delivery: {token}) {{
    match true {{
        true => delivery.ack(),
        false => delivery.nack(),
    }}
}}

#[kobo::scenario(profile = "async")]
fn {scenario}() {{
    let delivery = {token} {{}};
    ward.task();
    {finish}(delivery);
}}
"#
    );
    let file = project.main_file(&source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("both"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            scenario,
            s("--events=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_success(
        &output,
        "engine=both must execute compiler semantics and harness",
    );
    let witness = read_json(&first_witness(&project));
    assert_eq!(witness["replay_guarantee"].as_str(), Some("exact"));
    assert_eq!(
        witness["execution_digest"]["semantic_engine"].as_str(),
        Some("driver-kir-scenario")
    );
    assert_eq!(
        witness["execution_digest"]["harness_engine"].as_str(),
        Some("generated-rust-process")
    );
    assert_eq!(
        witness["execution_digest"]["agreement"].as_str(),
        Some("matched")
    );
    assert!(
        witness["execution_digest"]["generated_rust_hash"]
            .as_str()
            .is_some(),
        "exact witness must record generated Rust hash"
    );
    assert!(
        witness["execution_digest"]["harness_manifest_hash"]
            .as_str()
            .is_some(),
        "exact witness must record harness manifest hash"
    );
    assert_eq!(
        witness["execution_digest"]["harness_exit_code"].as_i64(),
        Some(0),
        "exact witness must record successful harness process"
    );
    assert_json_has_path(
        &witness,
        &["coverage", "unsupported_constructs"],
        "coverage must be explicit",
    );
    assert_eq!(
        witness["coverage"]["unsupported_constructs"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "exact replay cannot contain unsupported constructs"
    );
}

#[test]
fn unsupported_construct_downgrades_to_partial_not_exact() {
    let project = TestProject::new("full-depth-unsupported");
    let scenario = unique_symbol("unsupported_select");
    let source = format!(
        r#"
#[kobo::scenario(profile = "async")]
async fn {scenario}() {{
    tokio::select! {{
        _ = async {{ ward.task(); }} => {{}}
        _ = async {{ ward.time.now(); }} => {{}}
    }}
}}
"#
    );
    let file = project.main_file(&source);
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("both"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            scenario,
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );

    assert_failure(&output, "unsupported async select cannot be exact");
    assert_contains(&output.combined(), "K0116", "coverage gap must emit K0116");
    let witness = read_json(&first_witness(&project));
    assert_ne!(witness["replay_guarantee"].as_str(), Some("exact"));
    assert_contains(
        &witness["coverage"].to_string(),
        "tokio::select",
        "witness coverage must name the unsupported construct",
    );
}

#[test]
fn lsp_replay_action_uses_matching_witness_not_stale_guess() {
    let project = TestProject::new("full-depth-lsp-artifact");
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
    project.write(
        &format!(".kobo/witnesses/{scenario}-0.kwit"),
        r#"{"schema_version":1,"source":{"hash":"stale"},"target":"stale:file"}"#,
    );

    let sim = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("both"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            scenario.clone(),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&sim, "failing scenario should emit a real witness");
    let actual_witness = project
        .find_files_with_ext("kwit")
        .into_iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(&scenario))
                && fs::read_to_string(path)
                    .map(|text| text.contains(&format!("\"name\": \"{scenario}\"")))
                    .unwrap_or(false)
        })
        .expect("actual witness should exist");

    let lsp = run_kobo(
        &[
            s("lsp-diagnostics"),
            s("--format=json"),
            s("--actions"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&lsp, "lsp diagnostics should succeed");
    let text = lsp.combined();
    let expected_path = actual_witness.to_string_lossy().to_string();
    let expected_json_path = expected_path.replace('\\', "\\\\");
    assert_contains(
        &text,
        &expected_json_path,
        "LSP replay action must point at the matching witness artifact",
    );
    assert_not_contains(
        &text,
        ".kobo/witnesses/<witness>.kwit",
        "LSP must not expose placeholder replay paths",
    );
    assert_not_contains(
        &text,
        "stale:file",
        "LSP must ignore stale witness artifacts",
    );
}

#[test]
fn perf_real_evidence_uses_structured_session_not_source_estimate() {
    let project = TestProject::new("full-depth-perf-session");
    let file = project.main_file(
        r#"
fn main() {
    let mut values = Vec::new();
    values.push(1);
}
"#,
    );
    project.write(
        ".kobo/sessions/session-1/diagowner.jsonl",
        r#"{"binding":"values","borrow_count":51,"mut_borrow_count":37,"contention":11,"span":{"line":3,"column":9}}"#,
    );

    let output = run_kobo(
        &[
            s("perf"),
            s("--evidence"),
            s("real"),
            s("--session"),
            s(".kobo/sessions/session-1"),
            s("--format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(
        &output,
        "real perf evidence must read structured session artifact",
    );
    let json: Value = serde_json::from_str(output.stdout.trim()).expect("perf output must be JSON");
    assert_eq!(json["source"].as_str(), Some("session_diagowner"));
    assert_eq!(json["borrow_count"].as_u64(), Some(51));
    assert_eq!(json["mut_borrow_count"].as_u64(), Some(37));
    assert_eq!(json["contention"].as_u64(), Some(11));
}

#[test]
fn ownership_audit_uses_solver_provenance_not_source_text_tokens() {
    let project = TestProject::new("full-depth-ownership-audit");
    let file = project.main_file(
        r#"
fn main() {
    let text = ".clone() inside string is not debt";
    let comment = 1; // external::Client::new() in comment is not boundary
    println!("{}", text);
}
"#,
    );
    let output = run_kobo(
        &[s("inspect"), s("--audit"), s("json"), path_arg(&file)],
        &project.root,
    );
    assert_success(&output, "audit should run");
    let text = output.combined();
    assert_not_contains(
        &text,
        "mechanical",
        "string .clone() must not produce tier-1 debt",
    );
    assert_not_contains(
        &text,
        "external",
        "comment-only external path must not produce tier-3 debt",
    );
    assert_contains(
        &text,
        "provenance",
        "audit entries must carry solver/KIR provenance shape",
    );
}
