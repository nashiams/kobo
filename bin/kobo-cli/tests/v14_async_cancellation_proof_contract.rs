mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kobo_proof::{certificate_material_hash, core_material_hash, ProofCertificate};
use serde_json::Value;
use v09_common::{
    assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V14_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V14_TIMEOUT)
}

fn async_source(scenario_name: &str) -> String {
    format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let delivery = Delivery {{}};
    let retained = 7;
    helper().await;
    delivery.ack();
    let _after = retained;
}}
"#
    )
}

fn select_source(scenario_name: &str) -> String {
    format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let delivery = Delivery {{}};
    if std::env::var("KOBO_SELECT").is_ok() {{
        delivery.ack();
    }} else {{
        delivery.requeue();
    }}
}}
"#
    )
}

fn select_loser_owning_source() -> &'static str {
    r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "sync")]
fn select_loser_owning_case() {
    let delivery = Delivery {};
    if std::env::var("KOBO_SELECT").is_ok() {
        delivery.ack();
    } else {
        let _still_owned = 1;
    }
}
"#
}

fn spawned_task_source() -> &'static str {
    r#"
#[kobo::scenario(profile = "async")]
async fn spawned_task_requires_policy_case() {
    let task = tokio::spawn(async {});
    let _kept = task;
}
"#
}

fn timeout_source(scenario_name: &str) -> String {
    format!(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let delivery = Delivery {{}};
    let _result = tokio::time::timeout(std::time::Duration::from_secs(1), helper()).await;
    delivery.requeue();
}}
"#
    )
}

fn emit_artifact(project: &TestProject, source: &str, target: &str) -> PathBuf {
    let file = project.main_file(source);
    let artifact_path = project.root.join(format!("{target}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s(target),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&output, "proof emit should produce a valid artifact");
    artifact_path
}

fn emit_fails(project: &TestProject, source: &str, target: &str, expected: &str) {
    let file = project.main_file(source);
    let artifact_path = project.root.join(format!("{target}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&file),
            s("--target"),
            s(target),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_failure(&output, "proof emit should reject unsafe async ownership");
    assert!(
        output.combined().contains(expected),
        "rejection should mention `{expected}`\n{}",
        output.combined()
    );
}

fn read_value(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

fn rewrite_valid_certificate(path: &Path, mutate: impl FnOnce(&mut Value)) {
    let mut value = read_value(path);
    mutate(&mut value);
    let mut certificate: ProofCertificate =
        serde_json::from_value(value).expect("tampered certificate should remain schema-valid");
    certificate.core.hash = core_material_hash(
        &certificate.core.version,
        &certificate.core.cfg_nodes,
        &certificate.core.cfg_edges,
        &certificate.core.async_model,
    )
    .expect("core hash should recompute");
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should recompute");
    fs::write(path, serde_json::to_string_pretty(&certificate).unwrap())
        .expect("artifact should write");
}

fn verify_fails(project: &TestProject, artifact_path: &Path, expected: &str) {
    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(artifact_path)],
        &project.root,
    );
    assert_failure(&output, "tampered proof artifact must be rejected");
    assert!(
        output.combined().contains(expected),
        "rejection should mention `{expected}`\n{}",
        output.combined()
    );
}

fn async_model<'a>(artifact: &'a Value) -> &'a Value {
    artifact
        .pointer("/core/async_model")
        .expect("certificate should expose core async_model evidence")
}

#[test]
fn async_function_lowers_to_suspension_states() {
    let project = TestProject::new("v14-async-suspension");
    let artifact_path = emit_artifact(
        &project,
        &async_source("async_function_lowers_case"),
        "async_function_lowers_case",
    );
    let artifact = read_value(&artifact_path);
    let suspension_states = async_model(&artifact)["suspension_states"]
        .as_array()
        .expect("suspension states should be an array");

    assert!(
        suspension_states.iter().any(|state| {
            state["terminator_kind"] == "await"
                && state["resume_edge"].as_str() == Some("await_resume")
                && state["cancel_edge"].as_str() == Some("await_cancel")
        }),
        "await lowering should record resume and cancel states: {suspension_states:?}"
    );
}

#[test]
fn locals_live_across_await_become_future_state() {
    let project = TestProject::new("v14-async-future-locals");
    let artifact_path = emit_artifact(
        &project,
        &async_source("locals_live_across_await_case"),
        "locals_live_across_await_case",
    );
    let artifact = read_value(&artifact_path);
    let locals = async_model(&artifact)["future_state_locals"]
        .as_array()
        .expect("future state locals should be an array");

    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("retained")
                && local["suspension_state"].as_str().is_some()
        }),
        "retained local should be materialized in future state: {locals:?}"
    );
}

#[test]
fn obligations_live_across_await_attach_to_future_state() {
    let project = TestProject::new("v14-async-future-obligations");
    let artifact_path = emit_artifact(
        &project,
        &async_source("obligations_live_across_await_case"),
        "obligations_live_across_await_case",
    );
    let artifact = read_value(&artifact_path);
    let obligations = async_model(&artifact)["future_state_obligations"]
        .as_array()
        .expect("future state obligations should be an array");

    assert!(
        obligations.iter().any(|obligation| {
            obligation["binding"].as_str() == Some("delivery")
                && obligation["state"].as_str() == Some("owned")
        }),
        "owned delivery should be attached to the future state before await: {obligations:?}"
    );
}

#[test]
fn dropping_future_creates_cancel_edge() {
    let project = TestProject::new("v14-async-cancel-edges");
    let artifact_path = emit_artifact(
        &project,
        &async_source("dropping_future_creates_cancel_edge_case"),
        "dropping_future_creates_cancel_edge_case",
    );
    let artifact = read_value(&artifact_path);
    let cancel_edges = async_model(&artifact)["cancel_edges"]
        .as_array()
        .expect("cancel edges should be an array");

    assert!(
        cancel_edges.iter().any(|edge| {
            edge["reason"].as_str() == Some("future_drop")
                && edge["to"].as_str() == Some("await_cancel")
        }),
        "await suspension should expose a future-drop cancel edge: {cancel_edges:?}"
    );
}

#[test]
fn select_loser_owning_delivery_fails() {
    let project = TestProject::new("v14-select-loser-owned");

    emit_fails(
        &project,
        select_loser_owning_source(),
        "select_loser_owning_case",
        "unresolved obligation",
    );
}

#[test]
fn select_loser_with_explicit_requeue_passes() {
    let project = TestProject::new("v14-select-explicit-requeue");
    let artifact_path = emit_artifact(
        &project,
        &select_source("select_loser_requeue_case"),
        "select_loser_requeue_case",
    );
    let artifact = read_value(&artifact_path);
    let select_paths = async_model(&artifact)["select_paths"]
        .as_array()
        .expect("select paths should be an array");

    assert!(
        select_paths.iter().any(|path| {
            path["path_kind"].as_str() == Some("loser_cancel")
                && path["obligation_result_hash"].as_str().is_some()
        }),
        "select loser path should have explicit cancellation evidence: {select_paths:?}"
    );
}

#[test]
fn spawned_task_owning_delivery_requires_join_abort_detach_or_transfer() {
    let project = TestProject::new("v14-spawned-task-policy");

    emit_fails(
        &project,
        spawned_task_source(),
        "spawned_task_requires_policy_case",
        "spawned_task_obligations.resolution_event",
    );
}

#[test]
fn timeout_branch_models_cancellation() {
    let project = TestProject::new("v14-timeout-cancel");
    let artifact_path = emit_artifact(
        &project,
        &timeout_source("timeout_branch_models_cancellation_case"),
        "timeout_branch_models_cancellation_case",
    );
    let artifact = read_value(&artifact_path);
    let timeout_edges = async_model(&artifact)["timeout_cancel_edges"]
        .as_array()
        .expect("timeout cancel edges should be an array");

    assert!(
        timeout_edges.iter().any(|edge| {
            edge["source"].as_str() == Some("tokio::time::timeout")
                && edge["cancel_edge"].as_str() == Some("await_cancel")
        }),
        "timeout await should expose timeout cancellation evidence: {timeout_edges:?}"
    );
}

#[test]
fn removed_timeout_cancel_evidence_is_rejected_after_hash_recompute() {
    let project = TestProject::new("v14-timeout-tamper-model");
    let artifact_path = emit_artifact(
        &project,
        &timeout_source("removed_timeout_cancel_evidence_case"),
        "removed_timeout_cancel_evidence_case",
    );
    rewrite_valid_certificate(&artifact_path, |artifact| {
        artifact["core"]["async_model"]["timeout_cancel_edges"] = Value::Array(Vec::new());
    });

    verify_fails(&project, &artifact_path, "timeout_cancel_edges");
}

#[test]
fn reordered_branches_and_renamed_locals_preserve_obligation_result() {
    let project = TestProject::new("v14-select-hash-stability");
    let first_path = emit_artifact(
        &project,
        &select_source("select_hash_first_case"),
        "select_hash_first_case",
    );
    let second_path = emit_artifact(
        &project,
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "sync")]
fn select_hash_second_case() {
    let parcel = Delivery {};
    if std::env::var("KOBO_SELECT").is_err() {
        parcel.requeue();
    } else {
        parcel.ack();
    }
}
"#,
        "select_hash_second_case",
    );

    let first = read_value(&first_path);
    let second = read_value(&second_path);
    let first_hash = &async_model(&first)["select_paths"][0]["obligation_result_hash"];
    let second_hash = &async_model(&second)["select_paths"][0]["obligation_result_hash"];

    assert_eq!(
        first_hash, second_hash,
        "alpha-renamed and reordered select branches should preserve obligation result hashes"
    );
}

#[test]
fn removed_async_cancel_evidence_is_rejected() {
    let project = TestProject::new("v14-async-tamper-model");
    let artifact_path = emit_artifact(
        &project,
        &async_source("removed_async_cancel_evidence_case"),
        "removed_async_cancel_evidence_case",
    );
    rewrite_valid_certificate(&artifact_path, |artifact| {
        artifact["core"]["async_model"]["cancel_edges"] = Value::Array(Vec::new());
    });

    verify_fails(&project, &artifact_path, "missing async cancel evidence");
}
