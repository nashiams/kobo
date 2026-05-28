mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_test_support::{
    assert_failure, assert_success, first_json, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};
use kobo_proof::{certificate_material_hash, ProofCertificate};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn queue_loop_source(scenario_name: &str) -> String {
    format!(
        r#"
struct Queue {{}}

#[kobo::must_call(ack | nack | requeue)]
struct Delivery {{}}

impl Queue {{
    fn recv(&self) -> Delivery {{ Delivery {{}} }}
}}

impl Delivery {{
    fn ack(self) {{}}
    fn nack(self) {{}}
    fn requeue(self) {{}}
}}

#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let queue = Queue {{}};
    loop {{
        let delivery = queue.recv();
        delivery.ack();
    }}
}}
"#
    )
}

fn queue_loop_source_with_invariant(scenario_name: &str) -> String {
    queue_loop_source(scenario_name).replace(
        "#[kobo::scenario(profile = \"sync\")]",
        "#[kobo::invariant(expression = \"no_pending(Delivery)\")]\n#[kobo::scenario(profile = \"sync\")]",
    )
}

fn bounded_source(scenario_name: &str, completeness: &str, observed: u64, expected: u64) -> String {
    format!(
        r#"
#[kobo::bounded(histories = "{observed}", expected = "{expected}", completeness = "{completeness}", scheduler = "ready_queue_order", fault = "timeout", cancellation = "await-recv", queue_capacity = "1", message_count = "1", retry_attempts = "1", timeout_paths = "1", external_boundary_recordings = "0")]
#[kobo::scenario(profile = "sync")]
fn {scenario_name}() {{
    let _unit = ();
}}
"#
    )
}

fn emit_artifact(project: &TestProject, source: &str, scenario_name: &str) -> PathBuf {
    let source_file = project.main_file(source);
    let artifact_path = project.root.join(format!("{scenario_name}.kproof"));
    let output = run_kobo(
        &[
            s("proof"),
            s("emit"),
            path_arg(&source_file),
            s("--target"),
            s(scenario_name),
            s("--output"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );
    assert_success(&output, "proof emit should succeed");
    artifact_path
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap()).expect("artifact should write");
}

#[test]
fn proof_emit_records_loop_invariant_and_translation_trace_coverage() {
    let project = TestProject::new("translation-loop-trace-coverage");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("queue_loop_case"),
        "queue_loop_case",
    );
    let artifact = read_json(&artifact_path);

    assert!(
        artifact["core"]["loop_facts"]
            .as_array()
            .is_some_and(|facts| !facts.is_empty()),
        "loop back-edge facts must be in Core evidence: {artifact}"
    );
    let invariant = artifact["loop_invariants"][0].clone();
    assert_eq!(invariant["tier"], "inferred");
    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["template"]["id"], "queue_delivery");
    assert_eq!(invariant["template"]["version"], "0.1");
    assert_eq!(invariant["preservation"], "preserved");

    let core_trace = artifact["core_obligation_trace"]
        .as_array()
        .expect("Core trace must be an array");
    assert!(
        core_trace
            .iter()
            .any(|event| event["kind"] == "create" && event["binding"] == "delivery"),
        "Core trace must include Delivery create: {artifact}"
    );
    assert!(
        core_trace
            .iter()
            .any(|event| event["kind"] == "discharge" && event["binding"] == "delivery"),
        "Core trace must include Delivery discharge: {artifact}"
    );

    let generated_trace = artifact["generated_rust_trace"]
        .as_array()
        .expect("generated trace must be an array");
    assert_eq!(generated_trace.len(), core_trace.len());
    assert!(
        artifact["trace_hashes"]
            .as_array()
            .is_some_and(|hashes| hashes
                .iter()
                .any(|hash| hash["id"] == "core_obligation_trace")
                && hashes
                    .iter()
                    .any(|hash| hash["id"] == "generated_rust_trace")),
        "certificate must record stable hashes for both trace sides: {artifact}"
    );
    assert!(
        generated_trace.iter().all(|event| {
            event["source_map_anchor"]["status"] == "mapped"
                && event["source_map_anchor"]["id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        }),
        "generated trace events must carry real source-map anchors: {artifact}"
    );
    assert_eq!(artifact["translation_validation"]["status"], "validated");
}

#[test]
fn proof_verify_json_and_text_report_translation_validation_status() {
    let project = TestProject::new("translation-proof-verify-status");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("translation_status_case"),
        "translation_status_case",
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
    assert_eq!(json["translation_validation_status"], "validated");

    let text_output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );
    assert_success(&text_output, "proof verify should succeed");
    assert!(
        text_output.combined().contains("translation validated"),
        "text output should name trace preservation, not just green status: {}",
        text_output.combined()
    );
}

#[test]
fn proof_emit_records_user_invariant_coverage() {
    let project = TestProject::new("translation-user-invariant-coverage");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source_with_invariant("user_invariant_coverage_case"),
        "user_invariant_coverage_case",
    );
    let artifact = read_json(&artifact_path);
    let invariant = &artifact["loop_invariants"][0];

    assert_eq!(invariant["tier"], "user");
    assert_eq!(invariant["expression"], "no_pending(Delivery)");
    assert_eq!(invariant["preservation"], "preserved");
    assert!(
        invariant["source_span"]["mapped"].as_bool() == Some(true),
        "user invariant coverage must include source span: {artifact}"
    );
}

#[test]
fn sampled_bounded_history_reports_evidence_only() {
    let project = TestProject::new("translation-bounded-evidence");
    let artifact_path = emit_artifact(
        &project,
        &bounded_source("sampled_bounded_case", "sampled", 128, 384),
        "sampled_bounded_case",
    );
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["bounded_evidence"][0]["completeness"], "sampled");
    assert!(
        artifact["bounded_evidence"][0]["wording"]
            .as_str()
            .is_some_and(|wording| wording.contains("evidence only")),
        "sampled histories must not claim bounded proof: {artifact}"
    );
}

#[test]
fn dropped_generated_discharge_event_is_rejected() {
    let project = TestProject::new("translation-trace-tamper");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("trace_tamper_case"),
        "trace_tamper_case",
    );
    let mut artifact = read_json(&artifact_path);
    let generated_trace = artifact["generated_rust_trace"]
        .as_array_mut()
        .expect("generated trace should be mutable");
    generated_trace.retain(|event| event["kind"] != "discharge");
    write_json(&artifact_path, &artifact);

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "proof verify should reject dropped generated discharge events",
    );
    assert!(
        output.combined().contains("missing generated event"),
        "rejection should name the trace preservation failure: {}",
        output.combined()
    );
}

#[test]
fn stale_trace_hash_is_rejected_even_when_certificate_hash_is_recomputed() {
    let project = TestProject::new("translation-trace-hash-tamper");
    let artifact_path = emit_artifact(
        &project,
        &queue_loop_source("trace_hash_tamper_case"),
        "trace_hash_tamper_case",
    );
    let mut certificate: ProofCertificate =
        serde_json::from_value(read_json(&artifact_path)).expect("certificate should deserialize");
    certificate.trace_hashes[0].hash = "stale".to_owned();
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

    assert_failure(&output, "stale trace hash should reject proof verification");
    assert!(
        output.combined().contains("trace hash mismatch"),
        "rejection should name stale trace hashes: {}",
        output.combined()
    );
}
