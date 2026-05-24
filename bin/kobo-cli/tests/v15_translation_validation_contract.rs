mod v09_common;
mod v15_common;

use serde_json::json;
use v15_common::{
    assert_failure, assert_success, emit_artifact, path_arg, queue_loop_source, read_json,
    run_kobo, s, write_json, TestProject,
};

fn emit_queue_artifact(project: &TestProject, scenario_name: &str) -> std::path::PathBuf {
    emit_artifact(
        project,
        &queue_loop_source(scenario_name, "delivery", "queue"),
        scenario_name,
    )
}

#[test]
fn generated_rust_trace_matches_core_trace_and_validates() {
    let project = TestProject::new("v15-translation-valid");
    let artifact_path = emit_queue_artifact(&project, "translation_valid_case");
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["translation_validation"]["status"], "validated");
    assert_eq!(
        artifact["generated_rust_trace"]
            .as_array()
            .expect("generated trace should be an array")
            .len(),
        artifact["core_obligation_trace"]
            .as_array()
            .expect("Core trace should be an array")
            .len()
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&output, "valid generated trace should verify");
}

#[test]
fn dropped_discharge_event_fails_translation_validation() {
    let project = TestProject::new("v15-translation-dropped-discharge");
    let artifact_path = emit_queue_artifact(&project, "translation_drop_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable")
        .retain(|event| event["kind"] != "discharge");
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "dropped discharge should reject proof");
    assert!(
        output.combined().contains("missing generated event"),
        "failure should name missing generated event: {}",
        output.combined()
    );
}

#[test]
fn inserted_escape_event_fails_translation_validation() {
    let project = TestProject::new("v15-translation-extra-event");
    let artifact_path = emit_queue_artifact(&project, "translation_extra_case");
    let mut artifact = read_json(&artifact_path);
    let mut extra = artifact["generated_rust_trace"][0].clone();
    extra["id"] = "generated-extra-escape".into();
    extra["core_event_id"] = "core-extra-escape".into();
    extra["kind"] = "escape".into();
    extra["order"] = json!(99);
    artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable")
        .push(extra);
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "inserted generated event should reject proof");
    assert!(
        output.combined().contains("extra generated event"),
        "failure should name extra generated event: {}",
        output.combined()
    );
}

#[test]
fn meaning_changing_reorder_fails_translation_validation() {
    let project = TestProject::new("v15-translation-reorder");
    let artifact_path = emit_queue_artifact(&project, "translation_reorder_case");
    let mut artifact = read_json(&artifact_path);
    let generated = artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable");
    generated[0]["order"] = json!(1);
    generated[1]["order"] = json!(0);
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "meaning-changing trace reorder should reject proof",
    );
    assert!(
        output.combined().contains("order mismatch"),
        "failure should name order mismatch: {}",
        output.combined()
    );
}

#[test]
fn stale_source_map_anchor_fails_translation_validation() {
    let project = TestProject::new("v15-translation-stale-anchor");
    let artifact_path = emit_queue_artifact(&project, "translation_stale_anchor_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"][0]["source_map_anchor"]["status"] = "stale".into();
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "stale source-map anchor should reject proof");
    assert!(
        output.combined().contains("source-map anchor"),
        "failure should name stale source-map anchor: {}",
        output.combined()
    );
}

#[test]
fn changed_template_version_fails_translation_validation() {
    let project = TestProject::new("v15-translation-template");
    let artifact_path = emit_queue_artifact(&project, "translation_template_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"][0]["template_version"] = "stale".into();
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "template mismatch should reject proof");
    assert!(
        output.combined().contains("template mismatch"),
        "failure should name template mismatch: {}",
        output.combined()
    );
}

#[test]
fn translation_status_cannot_be_toggled_without_trace_evidence() {
    let project = TestProject::new("v15-translation-status-toggle");
    let artifact_path = emit_queue_artifact(&project, "translation_status_toggle_case");
    let mut artifact = read_json(&artifact_path);
    artifact["generated_rust_trace"] = json!([]);
    artifact["translation_validation"]["status"] = "validated".into();
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "status toggle without trace evidence should reject proof",
    );
    assert!(
        output.combined().contains("missing generated event")
            || output
                .combined()
                .contains("translation validation status mismatch"),
        "failure should name trace evidence gap: {}",
        output.combined()
    );
}
