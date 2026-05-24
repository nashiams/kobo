mod v09_common;
mod v15_common;

use v15_common::{
    assert_success, bounded_source, emit_artifact, first_json, path_arg, read_json, run_kobo, s,
    TestProject,
};

#[test]
fn complete_finite_state_space_emits_bounded_proof_wording() {
    let project = TestProject::new("v15-bounded-complete");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "bounded_complete_case",
            "complete",
            384,
            384,
            Some("ready_queue_order"),
            Some("timeout_or_success"),
            Some("await_recv"),
        ),
        "bounded_complete_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["bounded_evidence"][0]["completeness"], "complete");
    assert_eq!(
        artifact["bounded_evidence"][0]["enumerated_history_count"],
        384
    );
    assert_eq!(
        artifact["bounded_evidence"][0]["wording"],
        "bounded proof: all 384 histories explored under declared bounds"
    );
}

#[test]
fn sampled_histories_emit_evidence_only_wording() {
    let project = TestProject::new("v15-bounded-sampled");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "bounded_sampled_case",
            "sampled",
            128,
            384,
            Some("ready_queue_order"),
            Some("timeout_or_success"),
            Some("await_recv"),
        ),
        "bounded_sampled_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["bounded_evidence"][0]["completeness"], "sampled");
    assert!(
        artifact["bounded_evidence"][0]["wording"]
            .as_str()
            .is_some_and(|wording| wording.contains("evidence only")),
        "sampled exploration must not claim bounded proof: {artifact}"
    );
}

#[test]
fn timeout_cutoff_cannot_claim_bounded_proof() {
    let project = TestProject::new("v15-bounded-timeout");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "bounded_timeout_case",
            "timeout",
            384,
            384,
            Some("ready_queue_order"),
            Some("timeout_or_success"),
            Some("await_recv"),
        ),
        "bounded_timeout_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["bounded_evidence"][0]["completeness"], "timeout");
    assert!(
        artifact["bounded_evidence"][0]["wording"]
            .as_str()
            .is_some_and(|wording| wording.contains("evidence only")),
        "timeout exploration must be evidence-only: {artifact}"
    );
}

#[test]
fn missing_scheduler_dimension_downgrades_to_evidence_only() {
    let project = TestProject::new("v15-bounded-missing-scheduler");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "bounded_missing_scheduler_case",
            "complete",
            384,
            384,
            None,
            Some("timeout_or_success"),
            Some("await_recv"),
        ),
        "bounded_missing_scheduler_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(
        artifact["bounded_evidence"][0]["completeness"],
        "incomplete"
    );
    assert!(
        artifact["bounded_evidence"][0]["wording"]
            .as_str()
            .is_some_and(|wording| wording.contains("evidence only")),
        "omitted proof-relevant dimensions must not claim bounded proof: {artifact}"
    );
}

#[test]
fn proof_verify_json_and_text_report_same_bounded_wording() {
    let project = TestProject::new("v15-bounded-cli-wording");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "bounded_cli_case",
            "complete",
            12,
            12,
            Some("single_thread"),
            Some("none"),
            Some("none"),
        ),
        "bounded_cli_case",
    );

    let json_output = run_kobo(
        &[
            s("proof"),
            s("verify"),
            s("--json"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&json_output, "proof verify --json should succeed");
    let json = first_json(&json_output, "proof verify json");
    assert_eq!(
        json["bounded_wording"][0],
        "bounded proof: all 12 histories explored under declared bounds"
    );

    let text_output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&text_output, "proof verify should succeed");
    assert!(
        text_output
            .combined()
            .contains("bounded proof: all 12 histories explored under declared bounds"),
        "text output should match JSON bounded wording: {}",
        text_output.combined()
    );
}
