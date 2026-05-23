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

fn sync_source(scenario_name: &str) -> String {
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
    delivery.ack();
}}
"#
    )
}

fn async_source() -> &'static str {
    r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

async fn helper() {}

#[kobo::scenario(profile = "async")]
async fn async_cancel_case() {
    let delivery = Delivery {};
    helper().await;
    delivery.ack();
}
"#
}

fn boundary_debt_source() -> &'static str {
    r#"
#[kobo::boundary(crate = "payments", policy = "debt", reason = "unverified adapter")]
use payments::charge;

#[kobo::scenario(profile = "sync")]
fn debt_boundary_case() {
    let _result = charge();
}
"#
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

fn read_value(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

fn write_value(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).expect("artifact should write");
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

fn rewrite_certificate_hash_only(path: &Path, mutate: impl FnOnce(&mut Value)) {
    let mut value = read_value(path);
    mutate(&mut value);
    let mut certificate: ProofCertificate =
        serde_json::from_value(value).expect("tampered certificate should remain schema-valid");
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should recompute");
    fs::write(path, serde_json::to_string_pretty(&certificate).unwrap())
        .expect("artifact should write");
}

#[test]
fn missing_cancel_edge_rejected() {
    let project = TestProject::new("v14-tamper-cancel-edge");
    let artifact_path = emit_artifact(&project, async_source(), "async_cancel_case");

    rewrite_valid_certificate(&artifact_path, |value| {
        let edges = value["core"]["cfg_edges"]
            .as_array_mut()
            .expect("cfg edges should be mutable");
        edges.retain(|edge| edge["to"].as_str() != Some("await_cancel"));
    });

    verify_fails(&project, &artifact_path, "missing cancel edge");
}

#[test]
fn cfg_edge_to_unknown_block_rejected_after_hash_recompute() {
    let project = TestProject::new("v14-tamper-cfg-transition");
    let artifact_path = emit_artifact(
        &project,
        &sync_source("cfg_transition_case"),
        "cfg_transition_case",
    );

    rewrite_valid_certificate(&artifact_path, |value| {
        value["core"]["cfg_edges"][0]["to"] = Value::String("bb999".to_owned());
    });

    verify_fails(&project, &artifact_path, "CFG edge transition mismatch");
}

#[test]
fn async_model_material_changes_require_core_hash_update() {
    let project = TestProject::new("v14-tamper-async-core-hash");
    let artifact_path = emit_artifact(&project, async_source(), "async_cancel_case");

    rewrite_certificate_hash_only(&artifact_path, |value| {
        value["core"]["async_model"]["suspension_states"][0]["resume_edge"] =
            Value::String("tampered_resume".to_owned());
    });

    verify_fails(&project, &artifact_path, "core hash mismatch");
}

#[test]
fn removed_future_state_obligation_rejected_after_hash_recompute() {
    let project = TestProject::new("v14-tamper-future-state-obligation");
    let artifact_path = emit_artifact(&project, async_source(), "async_cancel_case");

    rewrite_valid_certificate(&artifact_path, |value| {
        value["core"]["async_model"]["future_state_obligations"] = Value::Array(Vec::new());
    });

    verify_fails(&project, &artifact_path, "future state obligation");
}

#[test]
fn removed_discharge_rejected() {
    let project = TestProject::new("v14-tamper-discharge");
    let artifact_path = emit_artifact(
        &project,
        &sync_source("removed_discharge_case"),
        "removed_discharge_case",
    );

    rewrite_valid_certificate(&artifact_path, |value| {
        let events = value["obligation_events"]
            .as_array_mut()
            .expect("obligation events should be mutable");
        events.retain(|event| event["kind"].as_str() != Some("discharge"));
    });

    verify_fails(&project, &artifact_path, "removed discharge");
}

#[test]
fn stale_template_hash_rejected() {
    let project = TestProject::new("v14-tamper-template");
    let artifact_path = emit_artifact(
        &project,
        &sync_source("stale_template_case"),
        "stale_template_case",
    );
    let mut artifact = read_value(&artifact_path);
    artifact["template_hashes"][0]["hash"] = Value::String("stale-template".to_owned());
    write_value(&artifact_path, &artifact);

    verify_fails(&project, &artifact_path, "template hash mismatch");
}

#[test]
fn unknown_boundary_policy_rejected() {
    let project = TestProject::new("v14-tamper-policy");
    let artifact_path = emit_artifact(&project, boundary_debt_source(), "debt_boundary_case");
    let mut artifact = read_value(&artifact_path);
    artifact["boundary_assumptions"][0]["policy"] = Value::String("mystery".to_owned());
    write_value(&artifact_path, &artifact);

    verify_fails(&project, &artifact_path, "unknown boundary policy");
}

#[test]
fn unknown_event_kind_rejected() {
    let project = TestProject::new("v14-tamper-event");
    let artifact_path = emit_artifact(
        &project,
        &sync_source("unknown_event_case"),
        "unknown_event_case",
    );
    let mut artifact = read_value(&artifact_path);
    artifact["obligation_events"][0]["kind"] = Value::String("teleport".to_owned());
    write_value(&artifact_path, &artifact);

    verify_fails(&project, &artifact_path, "unknown obligation event kind");
}

#[test]
fn unknown_unversioned_field_rejected() {
    let project = TestProject::new("v14-tamper-field");
    let artifact_path = emit_artifact(
        &project,
        &sync_source("unknown_field_case"),
        "unknown_field_case",
    );
    let mut artifact = read_value(&artifact_path);
    artifact["future_unversioned_field"] = Value::Bool(true);
    write_value(&artifact_path, &artifact);

    verify_fails(
        &project,
        &artifact_path,
        "unknown unversioned certificate field",
    );
}

#[test]
fn exact_over_disallowed_boundary_rejected() {
    let project = TestProject::new("v14-tamper-exact-boundary");
    let artifact_path = emit_artifact(&project, boundary_debt_source(), "debt_boundary_case");

    rewrite_valid_certificate(&artifact_path, |value| {
        value["replay_grade"] = Value::String("exact".to_owned());
        value["coverage_loss"] = Value::Array(Vec::new());
    });

    verify_fails(
        &project,
        &artifact_path,
        "exact replay crosses disallowed boundary",
    );
}

#[test]
fn exact_replay_with_coverage_loss_rejected() {
    let project = TestProject::new("v14-tamper-exact-coverage-loss");
    let artifact_path = emit_artifact(
        &project,
        &sync_source("exact_coverage_loss_case"),
        "exact_coverage_loss_case",
    );

    rewrite_valid_certificate(&artifact_path, |value| {
        value["replay_grade"] = Value::String("exact".to_owned());
        value["coverage_loss"] = serde_json::json!([{
            "kind": "unsupported_construct",
            "label": "manual-tamper",
            "reason": "coverage loss must block exact replay"
        }]);
    });

    verify_fails(&project, &artifact_path, "exact replay has coverage loss");
}
