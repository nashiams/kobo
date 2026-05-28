mod cli_test_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cli_test_support::{
    assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn candidate_source(target: &str, candidate_attr: &str) -> String {
    format!(
        r#"
{candidate_attr}
#[kobo::scenario(profile = "sync")]
fn {target}() {{
    let _value = 1;
}}
"#
    )
}

fn complete_candidate_attr() -> &'static str {
    r#"#[kobo::candidate_track(
    id = "S-32",
    track = "MSRV-aware emission",
    status = "research",
    inspect = "inspect shows target Rust lowering",
    manual_rust = "write the same module using stable Rust 1.70 syntax",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_candidate_msrv_snapshot",
    replay_related = "false"
)]"#
}

fn replay_candidate_attr() -> &'static str {
    r#"#[kobo::candidate_track(
    id = "research-adapter",
    track = "Replay adapter research",
    status = "research",
    inspect = "inspect shows adapter confidence",
    manual_rust = "call the adapter behind an explicit boundary policy",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_candidate_adapter_snapshot",
    replay_related = "true"
)]"#
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
    assert_success(&output, "proof emit should produce candidate evidence");
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
    assert_failure(
        &output,
        "candidate admission gate should reject missing evidence",
    );
    assert!(
        output.combined().contains(expected),
        "rejection should mention `{expected}`\n{}",
        output.combined()
    );
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("artifact should read"))
        .expect("artifact should parse")
}

#[test]
fn candidate_requires_inspect_visibility() {
    let project = TestProject::new("proof-candidate-inspect");
    let file = project.main_file(&candidate_source(
        "candidate_inspect_case",
        complete_candidate_attr(),
    ));

    let output = run_kobo(&[s("inspect"), s("--sim"), path_arg(&file)], &project.root);

    assert_success(&output, "inspect should expose candidate metadata");
    assert!(
        output.combined().contains("candidate_admission"),
        "inspect output should expose candidate admission metadata: {}",
        output.combined()
    );
    assert!(
        output.combined().contains("inspect_visible=true"),
        "inspect output should name the inspect visibility gate: {}",
        output.combined()
    );
}

#[test]
fn candidate_requires_manual_rust_equivalent() {
    let project = TestProject::new("proof-candidate-manual");
    let source = candidate_source(
        "candidate_manual_case",
        r#"#[kobo::candidate_track(
    id = "S-32",
    track = "MSRV-aware emission",
    status = "graduate",
    inspect = "inspect shows target Rust lowering",
    strict = "true",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_candidate_msrv_snapshot",
    replay_related = "false"
)]"#,
    );

    emit_fails(
        &project,
        &source,
        "candidate_manual_case",
        "manual Rust equivalent",
    );
}

#[test]
fn candidate_requires_strict_compatibility() {
    let project = TestProject::new("proof-candidate-strict");
    let source = candidate_source(
        "candidate_strict_case",
        r#"#[kobo::candidate_track(
    id = "S-46",
    track = "Proc-macro replacement DSL",
    status = "graduate",
    inspect = "inspect shows builder transform",
    manual_rust = "write the builder by hand",
    strict = "false",
    whole_ecosystem = "false",
    diagnostic_snapshot = "proof_candidate_proc_macro_snapshot",
    replay_related = "false"
)]"#,
    );

    emit_fails(
        &project,
        &source,
        "candidate_strict_case",
        "Strict-compatible",
    );
}

#[test]
fn candidate_rejects_whole_ecosystem_modeling_requirement() {
    let project = TestProject::new("proof-candidate-ecosystem");
    let source = candidate_source(
        "candidate_ecosystem_case",
        r#"#[kobo::candidate_track(
    id = "research-adapters",
    track = "Broad ecosystem adapters",
    status = "graduate",
    inspect = "inspect shows curated adapter boundary",
    manual_rust = "write explicit boundary policies for selected crates",
    strict = "true",
    whole_ecosystem = "true",
    diagnostic_snapshot = "proof_candidate_adapter_snapshot",
    replay_related = "true"
)]"#,
    );

    emit_fails(
        &project,
        &source,
        "candidate_ecosystem_case",
        "whole-ecosystem modeling",
    );
}

#[test]
fn candidate_requires_diagnostic_snapshots() {
    let project = TestProject::new("proof-candidate-diagnostics");
    let source = candidate_source(
        "candidate_diagnostics_case",
        r#"#[kobo::candidate_track(
    id = "S-37",
    track = "Numeric cast refinement",
    status = "graduate",
    inspect = "inspect shows explicit cast policy",
    manual_rust = "use saturating or checked conversions manually",
    strict = "true",
    whole_ecosystem = "false",
    replay_related = "false"
)]"#,
    );

    emit_fails(
        &project,
        &source,
        "candidate_diagnostics_case",
        "diagnostic snapshot",
    );
}

#[test]
fn candidate_requires_replay_grade_and_adapter_confidence_when_replay_related() {
    let project = TestProject::new("proof-candidate-replay");
    let artifact_path = emit_artifact(
        &project,
        &candidate_source("candidate_replay_case", replay_candidate_attr()),
        "candidate_replay_case",
    );
    let artifact = read_json(&artifact_path);
    let candidate = &artifact["candidate_admission"][0];

    assert_eq!(candidate["replay_related"], true);
    assert!(
        candidate["replay_grade"].as_str().is_some(),
        "replay-related candidates must expose replay grade: {artifact}"
    );
    assert!(
        candidate["adapter_confidence"].as_array().is_some(),
        "replay-related candidates must expose adapter confidence: {artifact}"
    );
}
