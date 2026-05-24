mod v09_common;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kobo_proof::{certificate_material_hash, ArtifactKind, ProofCertificate};
use serde_json::Value;
use v09_common::{
    assert_failure, assert_success, first_json, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};

const V14_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V14_TIMEOUT)
}

fn source(scenario_name: &str) -> String {
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

fn emit_project(label: &str, scenario_name: &str) -> (TestProject, PathBuf, PathBuf) {
    let project = TestProject::new(label);
    let source_file = project.main_file(&source(scenario_name));
    let artifact_path = project.root.join("proof.kproof");
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
    (project, source_file, artifact_path)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

#[test]
fn proof_emit_writes_kproof_with_required_schema() {
    let (_project, _source_file, artifact_path) = emit_project("v14-proof-emit", "proof_emit_case");
    let artifact = read_json(&artifact_path);

    assert_eq!(artifact["schema_version"], 2);
    assert_eq!(artifact["artifact_kind"], "kproof");
    assert_eq!(artifact["semantic_schema"], ".kproof");
    assert_eq!(artifact["claim_scope"], "modeled_core_obligation_flow_only");
}

#[test]
fn proof_verify_accepts_valid_artifact() {
    let (project, _source_file, artifact_path) =
        emit_project("v14-proof-verify-valid", "proof_verify_valid_case");

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_success(&output, "proof verify should accept a valid artifact");
    assert!(
        output.combined().contains("verified"),
        "valid verification should report verified status: {}",
        output.combined()
    );
}

#[test]
fn proof_verify_rejects_tampered_artifact() {
    let (project, _source_file, artifact_path) =
        emit_project("v14-proof-verify-tampered", "proof_verify_tampered_case");
    let mut artifact = read_json(&artifact_path);
    artifact["core"]["hash"] = Value::String("tampered-core".to_owned());
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&artifact).unwrap(),
    )
    .expect("tampered artifact should write");

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(&output, "proof verify should reject tampered artifact");
    assert!(
        output.combined().contains("core hash"),
        "tamper rejection should name the failing check: {}",
        output.combined()
    );
}

#[test]
fn proof_verify_rejects_artifact_kind_path_mismatch() {
    let (project, _source_file, artifact_path) =
        emit_project("v14-proof-verify-kind-mismatch", "proof_verify_kind_case");
    let mut certificate: ProofCertificate =
        serde_json::from_str(&fs::read_to_string(&artifact_path).expect("artifact should read"))
            .expect("artifact should parse as certificate");
    certificate.artifact_kind = ArtifactKind::KwitProofJson;
    certificate.certificate_material_hash =
        certificate_material_hash(&certificate).expect("mismatched artifact should still hash");
    fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&certificate).unwrap(),
    )
    .expect("mismatched artifact should write");

    let output = run_kobo(
        &[s("proof"), s("verify"), path_arg(&artifact_path)],
        &project.root,
    );

    assert_failure(
        &output,
        "proof verify should reject certificate artifact_kind that disagrees with the file path",
    );
    assert!(
        output.combined().contains("artifact_kind"),
        "artifact kind mismatch should name the header field: {}",
        output.combined()
    );
}

#[test]
fn check_emit_proof_runs_check_and_emits_artifact() {
    let project = TestProject::new("v14-check-emit-proof");
    let source_file = project.main_file(&source("check_emit_case"));

    let output = run_kobo(
        &[s("check"), s("--emit-proof"), path_arg(&source_file)],
        &project.root,
    );

    assert_success(
        &output,
        "check --emit-proof should run check and emit proof",
    );
    let artifact_path = source_file.with_extension("kproof");
    let artifact = read_json(&artifact_path);
    assert_eq!(artifact["artifact_kind"], "kproof");
}

#[test]
fn check_emit_proof_refuses_exact_over_disallowed_boundary() {
    let project = TestProject::new("v14-check-emit-proof-boundary");
    let source_file = project.main_file(boundary_debt_source());

    let output = run_kobo(
        &[s("check"), s("--emit-proof=exact"), path_arg(&source_file)],
        &project.root,
    );

    assert_failure(
        &output,
        "check --emit-proof exact should reject debt boundaries",
    );
    assert!(
        output
            .combined()
            .contains("exact replay crosses disallowed boundary"),
        "exact proof refusal should name the policy gap: {}",
        output.combined()
    );
}

#[test]
fn proof_cli_json_output_is_machine_readable() {
    let (project, _source_file, artifact_path) = emit_project("v14-proof-json", "proof_json_case");

    let output = run_kobo(
        &[
            s("proof"),
            s("verify"),
            s("--json"),
            path_arg(&artifact_path),
        ],
        &project.root,
    );

    assert_success(&output, "proof verify --json should succeed");
    let json = first_json(&output, "proof verify json");
    assert_eq!(json["status"], "verified");
    assert!(
        json["checked_obligation_events"]
            .as_u64()
            .unwrap_or_default()
            > 0,
        "JSON output should expose verifier replay evidence: {json}"
    );
}
