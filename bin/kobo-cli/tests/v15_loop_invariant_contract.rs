mod v09_common;
mod v15_common;

use v15_common::{
    assert_failure, assert_success, branch_leak_loop_source, emit_artifact, path_arg,
    queue_loop_source, read_json, run_kobo, s, TestProject,
};

#[test]
fn queue_loop_infers_template_invariant() {
    let project = TestProject::new("v15-loop-invariant-queue");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("queue_invariant_case", "delivery", "queue"),
        "queue_invariant_case",
    );
    let artifact = read_json(&artifact_path);
    let invariant = &artifact["loop_invariants"][0];

    assert_eq!(invariant["tier"], "inferred");
    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["template"]["id"], "queue_delivery");
    assert_eq!(invariant["template"]["version"], "0.1");
    assert_eq!(invariant["preservation"], "preserved");
    assert!(
        invariant["back_edge_states"]
            .as_array()
            .is_some_and(|states| states
                .iter()
                .all(|state| state["state"] != "owned" && state["state"] != "branch_unresolved")),
        "back-edge must not carry unresolved per-iteration obligations: {artifact}"
    );
}

#[test]
fn renamed_queue_loop_proves_the_same_template() {
    let project = TestProject::new("v15-loop-invariant-renamed");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("renamed_queue_case", "message", "inbox"),
        "renamed_queue_case",
    );
    let artifact = read_json(&artifact_path);
    let invariant = &artifact["loop_invariants"][0];

    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["template"]["id"], "queue_delivery");
    assert_eq!(
        invariant["obligations_created"][0], "message",
        "binding rename should not change the inferred protocol template: {artifact}"
    );
}

#[test]
fn branch_leak_reaching_back_edge_rejects_proof() {
    let project = TestProject::new("v15-loop-invariant-branch-leak");
    let source_file = project.main_file(&branch_leak_loop_source("branch_leak_case"));
    let artifact_path = project.root.join("branch_leak_case.kproof");

    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s("branch_leak_case"),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );

    assert_failure(&output, "branch leak should reject proof emission");
    assert!(
        output.combined().contains("back-edge")
            || output.combined().contains("BranchUnresolved")
            || output.combined().contains("branch_unresolved")
            || output.combined().contains("CFG edge transition mismatch"),
        "failure should name the back-edge, join, or branch-unresolved obligation: {}",
        output.combined()
    );
}

#[test]
fn tampered_invariant_template_is_rejected() {
    let project = TestProject::new("v15-loop-invariant-template-tamper");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("template_tamper_case", "delivery", "queue"),
        "template_tamper_case",
    );
    let mut artifact = read_json(&artifact_path);
    artifact["loop_invariants"][0]["template"]["version"] = "stale".into();
    v15_common::write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "stale template must reject proof verification");
    assert!(
        output.combined().contains("template") || output.combined().contains("material hash"),
        "failure should name template evidence or certificate material: {}",
        output.combined()
    );
}

#[test]
fn valid_loop_artifact_verifies_after_invariant_checks() {
    let project = TestProject::new("v15-loop-invariant-verify");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("loop_verify_case", "delivery", "queue"),
        "loop_verify_case",
    );

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_success(&output, "valid loop invariant artifact should verify");
}
