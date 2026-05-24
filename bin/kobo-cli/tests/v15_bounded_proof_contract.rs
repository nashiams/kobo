mod v09_common;
mod v15_common;

use kobo_proof::{certificate_material_hash, ProofCertificate};
use v15_common::{
    assert_failure, assert_success, bounded_source, emit_artifact, first_json, path_arg, read_json,
    run_kobo, s, write_json, TestProject,
};

fn bounded_source_without_expected(
    scenario_name: &str,
    histories: u64,
    scheduler: &str,
    fault: &str,
    cancellation: &str,
) -> String {
    format!(
        r#"
#[kobo::bounded(histories = "{histories}", completeness = "complete", scheduler = "{scheduler}", fault = "{fault}", cancellation = "{cancellation}")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let _unit = ();
}}
"#
    )
}

fn bounded_source_with_unique_histories(
    scenario_name: &str,
    histories: u64,
    unique_histories: u64,
    expected: u64,
) -> String {
    format!(
        r#"
#[kobo::bounded(histories = "{histories}", unique_histories = "{unique_histories}", expected = "{expected}", completeness = "complete", scheduler = "ready_queue_order", fault = "timeout_or_success", cancellation = "await_recv")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let _unit = ();
}}
"#
    )
}

fn bounded_source_with_two_loops(scenario_name: &str) -> String {
    format!(
        r#"
#[kobo::bounded(histories = "1", expected = "1", completeness = "complete", scheduler = "single_thread", fault = "none", cancellation = "none")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    loop {{
        break;
    }}
    loop {{
        break;
    }}
}}
"#
    )
}

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
fn finite_state_space_can_derive_expected_count_from_declared_dimensions() {
    let project = TestProject::new("v15-bounded-derived-expected");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source_without_expected(
            "bounded_derived_expected_case",
            4,
            "fifo|round_robin",
            "none|timeout",
            "none",
        ),
        "bounded_derived_expected_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["bounded_evidence"][0]["completeness"], "complete");
    assert_eq!(
        artifact["bounded_evidence"][0]["expected_complete_history_count"],
        4
    );
    assert_eq!(
        artifact["bounded_evidence"][0]["wording"],
        "bounded proof: all 4 histories explored under declared bounds"
    );
}

#[test]
fn duplicate_histories_do_not_inflate_bounded_completeness() {
    let project = TestProject::new("v15-bounded-duplicate-histories");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source_with_unique_histories("bounded_duplicate_case", 384, 128, 384),
        "bounded_duplicate_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(
        artifact["bounded_evidence"][0]["completeness"],
        "incomplete"
    );
    assert_eq!(
        artifact["bounded_evidence"][0]["enumerated_history_count"],
        128
    );
    assert!(
        artifact["bounded_evidence"][0]["pruned_histories"]
            .as_array()
            .is_some_and(|histories| !histories.is_empty()),
        "duplicate history pruning must be recorded: {artifact}"
    );
    assert!(
        artifact["bounded_evidence"][0]["wording"]
            .as_str()
            .is_some_and(|wording| wording.contains("evidence only")),
        "duplicate-inflated histories must be evidence-only: {artifact}"
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

#[test]
fn complete_bounded_evidence_requires_exact_proof_wording() {
    let project = TestProject::new("v15-bounded-wording-tamper");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source(
            "bounded_wording_tamper_case",
            "complete",
            12,
            12,
            Some("single_thread"),
            Some("none"),
            Some("none"),
        ),
        "bounded_wording_tamper_case",
    );
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.bounded_evidence[0].wording = "bounded proof: trust me".to_owned();
    certificate.certificate_material_hash.clear();
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should compute");
    write_json(
        &artifact_path,
        &serde_json::to_value(&certificate).expect("certificate should serialize"),
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "tampered complete bounded wording should reject proof",
    );
    assert!(
        output.combined().contains("bounded evidence"),
        "failure should name bounded wording mismatch: {}",
        output.combined()
    );
}

#[test]
fn complete_bounded_evidence_names_every_loop_fact() {
    let project = TestProject::new("v15-bounded-loop-ids");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source_with_two_loops("bounded_loop_ids_case"),
        "bounded_loop_ids_case",
    );
    let artifact = read_json(&artifact_path);
    let loop_ids = artifact["bounded_evidence"][0]["loop_ids"]
        .as_array()
        .expect("bounded evidence should carry loop IDs");
    let fact_ids = artifact["core"]["loop_facts"]
        .as_array()
        .expect("Core evidence should carry loop facts");

    assert!(
        fact_ids.len() >= 2,
        "fixture should produce multiple concrete loop facts: {artifact}"
    );
    for fact in fact_ids {
        let fact_id = fact["id"].as_str().expect("loop fact should have an ID");
        assert!(
            loop_ids.iter().any(|loop_id| loop_id == fact_id),
            "bounded evidence must name loop fact {fact_id}: {artifact}"
        );
    }
}

#[test]
fn complete_bounded_evidence_missing_loop_id_cannot_cover_loop_fact() {
    let project = TestProject::new("v15-bounded-missing-loop-id");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source_with_two_loops("bounded_missing_loop_id_case"),
        "bounded_missing_loop_id_case",
    );
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.bounded_evidence[0].loop_ids.pop();
    certificate.certificate_material_hash.clear();
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("certificate hash should compute");
    write_json(
        &artifact_path,
        &serde_json::to_value(&certificate).expect("certificate should serialize"),
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "complete bounded evidence missing a loop ID should reject proof",
    );
    assert!(
        output
            .combined()
            .contains("missing a proof-grade back-edge fact"),
        "failure should name the uncovered loop fact: {}",
        output.combined()
    );
}
