mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_test_support::{
    assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use kobo_proof::{parse_certificate_json, verify_certificate, VerificationContext};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn source() -> &'static str {
    r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

impl Delivery {
    fn ack(self) {}
    fn nack(self) {}
    fn requeue(self) {}
}

#[kobo::scenario(profile = "sync")]
fn verifier_case() {
    let delivery = Delivery {};
    delivery.ack();
}
"#
}

fn emitted_artifact() -> (String, Value, String) {
    let project = TestProject::new("proof-proof-verifier");
    let file = project.main_file(source());
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--engine"),
            s("semantic"),
            s("--seed"),
            s("1402"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            s("--target"),
            s("verifier_case"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "verifier fixture should emit a proof artifact");
    let artifact_path = first_kwit_proof_path(&project);
    let artifact_source = fs::read_to_string(&artifact_path).expect("artifact should read");
    let artifact = serde_json::from_str(&artifact_source).expect("artifact should parse");
    let source_map_path = project
        .find_files_with_ext("map")
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name == "main.kobo.map"))
        .expect("test command should write a source map");
    let source_map = fs::read_to_string(source_map_path).expect("source map should read");
    (artifact_source, artifact, source_map)
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
        .expect("proof artifact should be emitted")
}

#[test]
fn emitted_certificate_verifies_independently() {
    let (artifact_source, artifact, source_map) = emitted_artifact();
    let certificate =
        parse_certificate_json(&artifact_source).expect("emitted artifact should be parseable");

    assert_eq!(artifact["translation_validation"]["status"], "validated");
    assert!(
        artifact["generated_rust_trace"]
            .as_array()
            .is_some_and(|events| !events.is_empty()),
        ".kwit proof artifact should carry generated trace evidence: {artifact}"
    );

    let report = verify_certificate(
        &certificate,
        &VerificationContext {
            source: source().to_owned(),
            source_map: Some(source_map),
        },
    )
    .expect("emitted artifact should verify independently");

    assert!(
        report.checked_obligation_events > 0,
        "verifier should replay obligation events, not only parse schema"
    );
}

#[test]
fn emitted_certificate_rejects_tampered_core_hash() {
    let (_artifact_source, mut artifact, source_map) = emitted_artifact();
    artifact["core"]["hash"] = Value::String("tampered-core".to_owned());
    let certificate =
        parse_certificate_json(&serde_json::to_string(&artifact).expect("artifact should render"))
            .expect("tampered artifact should still parse");

    let error = verify_certificate(
        &certificate,
        &VerificationContext {
            source: source().to_owned(),
            source_map: Some(source_map),
        },
    )
    .expect_err("tampered core hash must be rejected");

    assert!(
        error.to_string().contains("core hash"),
        "rejection should identify the failing check: {error}"
    );
}
