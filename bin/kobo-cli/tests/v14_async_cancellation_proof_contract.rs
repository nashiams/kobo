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

fn multi_await_source(scenario_name: &str) -> String {
    format!(
        r#"
async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let first = 1;
    helper().await;
    let second = first + 1;
    helper().await;
    let _after = first + second;
}}
"#
    )
}

fn same_statement_await_source(scenario_name: &str) -> String {
    format!(
        r#"
async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let first = 1;
    let _both = (helper().await, helper().await);
    let _after = first;
}}
"#
    )
}

fn same_expression_live_after_await_source(scenario_name: &str) -> String {
    format!(
        r#"
async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let first = 1;
    let _both = (helper().await, first);
}}
"#
    )
}

fn call_argument_await_source(scenario_name: &str) -> String {
    format!(
        r#"
async fn helper() {{}}
fn consume(_value: ()) {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let first = 1;
    consume(helper().await);
    let _after = first;
}}
"#
    )
}

fn block_expression_await_source(scenario_name: &str) -> String {
    format!(
        r#"
async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let first = 1;
    let _block = {{ helper().await }};
    let _after = first;
}}
"#
    )
}

fn if_expression_await_source(scenario_name: &str) -> String {
    format!(
        r#"
async fn helper() {{}}

#[kobo::scenario(profile = "async")]
async fn {scenario_name}() {{
    let first = 1;
    let _both = if std::env::var("KOBO_IF").is_ok() {{
        helper().await;
        first
    }} else {{
        first
    }};
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

fn assert_single_await_retains_first(artifact: &Value) {
    let suspensions = async_model(artifact)["suspension_states"]
        .as_array()
        .expect("suspension states should be an array");
    let cancel_edges = async_model(artifact)["cancel_edges"]
        .as_array()
        .expect("cancel edges should be an array");
    let locals = async_model(artifact)["future_state_locals"]
        .as_array()
        .expect("future state locals should be an array");

    assert_eq!(
        suspensions.len(),
        1,
        "one await should lower to one suspension: {suspensions:?}"
    );
    assert_eq!(
        cancel_edges.len(),
        1,
        "one await should expose one cancel edge: {cancel_edges:?}"
    );
    let suspension_id = suspensions[0]["id"]
        .as_str()
        .expect("suspension should have an id");
    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("first")
                && local["suspension_state"].as_str() == Some(suspension_id)
        }),
        "first should be future state across the await continuation: {locals:?}"
    );
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
fn locals_live_across_each_await_attach_to_the_matching_suspension() {
    let project = TestProject::new("v14-async-multi-await-future-locals");
    let artifact_path = emit_artifact(
        &project,
        &multi_await_source("multi_await_future_locals_case"),
        "multi_await_future_locals_case",
    );
    let artifact = read_value(&artifact_path);
    let locals = async_model(&artifact)["future_state_locals"]
        .as_array()
        .expect("future state locals should be an array");

    let first_suspension = async_model(&artifact)["suspension_states"][0]["id"]
        .as_str()
        .expect("first suspension should have an id");
    let second_suspension = async_model(&artifact)["suspension_states"][1]["id"]
        .as_str()
        .expect("second suspension should have an id");

    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("first")
                && local["suspension_state"].as_str() == Some(first_suspension)
        }),
        "first should live across the first await: {locals:?}"
    );
    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("first")
                && local["suspension_state"].as_str() == Some(second_suspension)
        }),
        "first should also live across the second await: {locals:?}"
    );
    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("second")
                && local["suspension_state"].as_str() == Some(second_suspension)
        }),
        "second is introduced after the first await and must live across the second await: {locals:?}"
    );
    assert!(
        !locals.iter().any(|local| {
            local["binding"].as_str() == Some("second")
                && local["suspension_state"].as_str() == Some(first_suspension)
        }),
        "second must not be attached to suspensions before it exists: {locals:?}"
    );
}

#[test]
fn same_statement_awaits_each_get_suspension_and_future_state() {
    let project = TestProject::new("v14-async-same-statement-awaits");
    let artifact_path = emit_artifact(
        &project,
        &same_statement_await_source("same_statement_await_case"),
        "same_statement_await_case",
    );
    let artifact = read_value(&artifact_path);
    let suspensions = async_model(&artifact)["suspension_states"]
        .as_array()
        .expect("suspension states should be an array");
    let cancel_edges = async_model(&artifact)["cancel_edges"]
        .as_array()
        .expect("cancel edges should be an array");
    let locals = async_model(&artifact)["future_state_locals"]
        .as_array()
        .expect("future state locals should be an array");

    assert_eq!(
        suspensions.len(),
        2,
        "each await expression should lower to a suspension: {suspensions:?}"
    );
    assert_eq!(
        cancel_edges.len(),
        2,
        "each await expression should have a cancel edge: {cancel_edges:?}"
    );
    for suspension in suspensions {
        let suspension_id = suspension["id"]
            .as_str()
            .expect("suspension should have an id");
        assert!(
            locals.iter().any(|local| {
                local["binding"].as_str() == Some("first")
                    && local["suspension_state"].as_str() == Some(suspension_id)
            }),
            "first should be live across each await expression: {locals:?}"
        );
    }
}

#[test]
fn same_expression_post_await_use_becomes_future_state() {
    let project = TestProject::new("v14-async-same-expression-post-await");
    let artifact_path = emit_artifact(
        &project,
        &same_expression_live_after_await_source("same_expression_post_await_case"),
        "same_expression_post_await_case",
    );
    let artifact = read_value(&artifact_path);
    let suspensions = async_model(&artifact)["suspension_states"]
        .as_array()
        .expect("suspension states should be an array");
    let locals = async_model(&artifact)["future_state_locals"]
        .as_array()
        .expect("future state locals should be an array");

    assert_eq!(suspensions.len(), 1, "one await should be modeled");
    let suspension_id = suspensions[0]["id"]
        .as_str()
        .expect("suspension should have an id");
    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("first")
                && local["suspension_state"].as_str() == Some(suspension_id)
        }),
        "first is used later in the same expression after await and must be future state: {locals:?}"
    );
}

#[test]
fn await_inside_helper_call_argument_lowers_to_core_suspension() {
    let project = TestProject::new("v14-async-helper-arg-await");
    let artifact_path = emit_artifact(
        &project,
        &call_argument_await_source("call_argument_await_case"),
        "call_argument_await_case",
    );
    let artifact = read_value(&artifact_path);
    let suspensions = async_model(&artifact)["suspension_states"]
        .as_array()
        .expect("suspension states should be an array");
    let cancel_edges = async_model(&artifact)["cancel_edges"]
        .as_array()
        .expect("cancel edges should be an array");
    let locals = async_model(&artifact)["future_state_locals"]
        .as_array()
        .expect("future state locals should be an array");

    assert_eq!(
        suspensions.len(),
        1,
        "await in helper-call argument should lower to a suspension: {suspensions:?}"
    );
    assert_eq!(
        cancel_edges.len(),
        1,
        "await in helper-call argument should expose cancellation: {cancel_edges:?}"
    );
    let suspension_id = suspensions[0]["id"]
        .as_str()
        .expect("suspension should have an id");
    assert!(
        locals.iter().any(|local| {
            local["binding"].as_str() == Some("first")
                && local["suspension_state"].as_str() == Some(suspension_id)
        }),
        "continuation after helper-call argument await should retain first: {locals:?}"
    );
}

#[test]
fn block_expression_await_keeps_later_locals_in_future_state() {
    let project = TestProject::new("v14-async-block-expression-await");
    let artifact_path = emit_artifact(
        &project,
        &block_expression_await_source("block_expression_await_case"),
        "block_expression_await_case",
    );
    assert_single_await_retains_first(&read_value(&artifact_path));
}

#[test]
fn if_expression_await_keeps_branch_continuation_locals_in_future_state() {
    let project = TestProject::new("v14-async-if-expression-await");
    let artifact_path = emit_artifact(
        &project,
        &if_expression_await_source("if_expression_await_case"),
        "if_expression_await_case",
    );
    assert_single_await_retains_first(&read_value(&artifact_path));
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
        "obligation replay mismatch",
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
    assert!(
        select_paths.iter().all(|path| {
            path["branch_target"].as_str().is_some()
                && path["obligation_results"].as_array().is_some_and(|states| {
                    states.iter().any(|state| {
                        state["binding"].as_str() == Some("delivery")
                            && state["state"].as_str() == Some("resolved")
                    })
                })
        }),
        "select evidence should be per branch target with obligation results: {select_paths:?}"
    );
    assert!(
        select_paths.iter().any(|path| {
            path["path_kind"].as_str() == Some("loser_cancel")
                && path["cancelled_obligations"]
                    .as_array()
                    .is_some_and(|states| states
                        .iter()
                        .any(|state| state["binding"].as_str() == Some("delivery")
                            && state["state"].as_str() == Some("owned")))
        }),
        "loser-cancel paths should carry the obligations canceled from the losing branch: {select_paths:?}"
    );
    let mut branch_targets = select_paths
        .iter()
        .filter_map(|path| path["branch_target"].as_str())
        .collect::<Vec<_>>();
    branch_targets.sort_unstable();
    branch_targets.dedup();
    assert!(
        branch_targets.len() >= 2,
        "select evidence should cover each branch target: {select_paths:?}"
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
fn removed_future_state_local_is_rejected_after_hash_recompute() {
    let project = TestProject::new("v14-future-local-tamper-model");
    let artifact_path = emit_artifact(
        &project,
        &async_source("removed_future_state_local_case"),
        "removed_future_state_local_case",
    );
    rewrite_valid_certificate(&artifact_path, |artifact| {
        artifact["core"]["async_model"]["future_state_locals"] = Value::Array(Vec::new());
    });

    verify_fails(&project, &artifact_path, "future_state_locals");
}

#[test]
fn removed_loser_cancel_obligation_evidence_is_rejected_after_hash_recompute() {
    let project = TestProject::new("v14-select-cancel-tamper-model");
    let artifact_path = emit_artifact(
        &project,
        &select_source("removed_loser_cancel_evidence_case"),
        "removed_loser_cancel_evidence_case",
    );
    rewrite_valid_certificate(&artifact_path, |artifact| {
        let paths = artifact["core"]["async_model"]["select_paths"]
            .as_array_mut()
            .expect("select paths should be mutable");
        for path in paths {
            if path["path_kind"].as_str() == Some("loser_cancel") {
                path["cancelled_obligations"] = Value::Array(Vec::new());
            }
        }
    });

    verify_fails(
        &project,
        &artifact_path,
        "select_paths.cancelled_obligations",
    );
}

#[test]
fn reordered_branches_and_renamed_locals_preserve_path_specific_results() {
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
    for artifact in [first, second] {
        let select_paths = async_model(&artifact)["select_paths"]
            .as_array()
            .expect("select paths should be an array");
        assert!(
            select_paths.iter().all(|path| {
                path["branch_target"].as_str().is_some()
                    && path["obligation_result_hash"].as_str().is_some()
                    && path["obligation_results"]
                        .as_array()
                        .is_some_and(|states| states
                            .iter()
                            .any(|state| state["state"].as_str() == Some("resolved")))
            }),
            "select evidence should stay path-specific after branch reorder/rename: {select_paths:?}"
        );
    }
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
