mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_test_support::{
    assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn proof_source(scenario_name: &str, action: &str) -> String {
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
    delivery.{action}();
}}
"#
    )
}

fn run_proof_artifact(project_label: &str, scenario_name: &str, action: &str) -> Value {
    let project = TestProject::new(project_label);
    let file = project.main_file(&proof_source(scenario_name, action));
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1401"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s(scenario_name),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "proof artifact fixture should run");
    let artifact_path = first_kwit_proof_path(&project);
    read_json(&artifact_path)
}

fn first_kwit_proof_path(project: &TestProject) -> PathBuf {
    project
        .find_files_with_ext("json")
        .into_iter()
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".kwit.proof.json"))
        })
        .expect("proof artifact should be emitted next to the witness")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("proof artifact should read"))
        .expect("proof artifact should parse as JSON")
}

#[test]
fn kproof_records_required_hashes_and_template_schemas() {
    let artifact = run_proof_artifact("proof-proof-required", "proof_required_case", "ack");

    assert_eq!(artifact["schema_version"], 2);
    assert_eq!(
        artifact["proof_target_version"],
        "kobo-core-obligation-flow-1"
    );
    assert_eq!(artifact["semantic_schema"], ".kproof");
    assert_eq!(artifact["artifact_kind"], "kwit.proof.json");
    assert!(
        artifact["source"]["hash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty()),
        "source hash must be recorded: {artifact}"
    );
    assert!(
        artifact["core"]["hash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty()),
        "Core hash must be recorded: {artifact}"
    );
    assert!(
        artifact["compiler_version"]
            .as_str()
            .is_some_and(|version| !version.is_empty()),
        "compiler version must be recorded: {artifact}"
    );
    assert!(
        artifact["template_hashes"]
            .as_array()
            .is_some_and(|hashes| !hashes.is_empty()),
        "template hashes must be recorded: {artifact}"
    );
    assert!(
        artifact["template_schemas"]
            .as_array()
            .is_some_and(|schemas| !schemas.is_empty()),
        "template schemas must be recorded: {artifact}"
    );
    let template_schema = &artifact["template_schemas"][0];
    assert_eq!(template_schema["template_schema"], "lifecycle-template");
    assert_eq!(template_schema["schema_version"], 1);
    assert!(
        template_schema.get("version").is_none(),
        "proof artifacts should not expose internal milestone template versions: {artifact}"
    );
    assert!(
        artifact["boundary_assumption_hashes"].as_array().is_some(),
        "boundary assumption hashes must be structurally present: {artifact}"
    );
}

#[test]
fn kproof_records_cfg_obligation_events_envs_summaries_and_coverage_loss() {
    let artifact = run_proof_artifact("proof-proof-cfg", "proof_cfg_case", "ack");

    assert!(
        artifact["core"]["cfg_nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty()),
        "CFG nodes must be structural: {artifact}"
    );
    assert!(
        artifact["core"]["cfg_edges"]
            .as_array()
            .is_some_and(|edges| !edges.is_empty()),
        "CFG edges must be structural: {artifact}"
    );
    let obligation_events = artifact["obligation_events"]
        .as_array()
        .expect("obligation events must be an array");
    assert!(
        obligation_events
            .iter()
            .any(|event| event["kind"].as_str() == Some("create")),
        "proof artifact must record create events: {artifact}"
    );
    assert!(
        obligation_events
            .iter()
            .any(|event| event["kind"].as_str() == Some("discharge")),
        "proof artifact must record discharge events: {artifact}"
    );
    assert!(
        artifact["entry_env"].is_array(),
        "entry env must be structural: {artifact}"
    );
    assert!(
        artifact["exit_env"].is_array(),
        "exit env must be structural: {artifact}"
    );
    assert!(
        artifact["function_summaries"]
            .as_array()
            .is_some_and(|summaries| !summaries.is_empty()),
        "function summaries must be structural: {artifact}"
    );
    assert!(
        artifact["coverage_loss"].as_array().is_some(),
        "coverage loss must be visible even when empty: {artifact}"
    );
}

#[test]
fn kproof_records_replay_grade_and_adapter_confidence() {
    let artifact = run_proof_artifact("proof-proof-grade", "proof_grade_case", "ack");

    assert_eq!(
        artifact["replay_grade"], "partial",
        "semantic-only proof fixtures must not overclaim exact replay"
    );
    assert!(
        artifact["adapter_confidence"].as_array().is_some(),
        "adapter confidence must be a structural certificate field: {artifact}"
    );
}

#[test]
fn kwit_proof_json_uses_same_semantic_schema() {
    let artifact = run_proof_artifact("proof-proof-kwit", "proof_kwit_case", "nack");

    assert_eq!(artifact["artifact_kind"], "kwit.proof.json");
    assert_eq!(artifact["semantic_schema"], ".kproof");
    assert!(
        artifact["certificate_material_hash"]
            .as_str()
            .is_some_and(|hash| !hash.is_empty()),
        ".kwit.proof.json must carry the same hashed semantic certificate material: {artifact}"
    );
}

#[test]
fn proof_artifact_claim_ends_at_core_until_translation_validation() {
    let artifact = run_proof_artifact("proof-proof-claim", "proof_claim_case", "requeue");

    assert_eq!(artifact["claim_scope"], "modeled_core_obligation_flow_only");
    let rendered = serde_json::to_string(&artifact).expect("artifact should render");
    assert!(
        !rendered.contains("generated Rust binary behavior"),
        "proof artifacts must not claim generated Rust binary behavior: {rendered}"
    );
}

#[test]
fn release_docs_state_scope_non_goals_and_honesty_gates() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("proof-artifacts.md");
    let docs = fs::read_to_string(&docs_path).expect("proof docs should exist");

    for required in [
        "independently checked proof artifacts for modeled obligation flow",
        "does not prove generated Rust binary behavior",
        "explicit non-goals",
        "adapter confidence",
        "replay grade",
        "candidate-track admission",
        "research tracks remain spikes",
        "template_schemas[]",
        "template_schema",
        "schema_version",
        "proof_target_version",
        "semantic_schema",
        "artifact_kind",
        "claim_scope",
        ".kproof",
        ".kwit.proof.json",
    ] {
        assert!(
            docs.contains(required),
            "docs should contain required release wording `{required}`:\n{docs}"
        );
    }

    for forbidden in [
        "proves arbitrary",
        "whole-program deterministic replay",
        "AI-native",
        "hidden runtime tax",
        "template versions",
        "template_versions",
    ] {
        assert!(
            !docs.contains(forbidden),
            "docs must not overclaim with `{forbidden}`:\n{docs}"
        );
    }
}
