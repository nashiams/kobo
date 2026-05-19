mod v09_common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_failure, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(60);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn json_diagnostic(output: &CliOutput) -> Value {
    serde_json::from_str(output.stdout.lines().next().unwrap_or_default())
        .unwrap_or_else(|error| panic!("diagnostic JSON should parse: {error}\n{}", output.stdout))
}

#[test]
fn k0100_liveness_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0100-snapshot");
    let file = project.main_file(
        r#"
#[kobo::must_call(ack | nack | requeue)]
struct Delivery {}

#[kobo::scenario(profile = "async")]
fn missing_ack() {
    let delivery = Delivery {};
    let _lost = delivery;
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "missing liveness action should fail");
    let diagnostic = json_diagnostic(&output);
    assert_eq!(diagnostic["code"], "K0100");
    assert_eq!(diagnostic["category"], "liveness");
    assert_contains(
        diagnostic["decision"].as_str().unwrap_or_default(),
        "Call",
        "K0100 decision should be actionable",
    );
}

#[test]
fn k0103_uncontrolled_effect_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0103-snapshot");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn crash_path() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--inject"),
            s("crash"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "crash injection should fail as uncontrolled effect",
    );
    let diagnostic = json_diagnostic(&output);
    assert_eq!(diagnostic["code"], "K0103");
    assert_eq!(diagnostic["category"], "replay");
    assert_contains(
        diagnostic["decision"].as_str().unwrap_or_default(),
        "Model",
        "K0103 decision should offer replay choices",
    );
}

#[test]
fn k0107_boundary_policy_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0107-snapshot");
    let file = project.copy_fixture("errors/boundary_replay_http.kobo", "src/main.kobo");

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "unconfigured boundary should fail");
    let diagnostic = json_diagnostic(&output);
    assert_eq!(diagnostic["code"], "K0107");
    for choice in ["model", "record", "outside", "opaque", "debt"] {
        assert_contains(
            &output.combined(),
            choice,
            "K0107 should expose boundary choices",
        );
    }
}

#[test]
fn k0108_trace_check_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0108-snapshot");
    let file = project.main_file(
        r#"
// kobo:temporal never deterministic-task
#[kobo::scenario(profile = "async")]
fn temporal_failure() {
    ward.task();
}
"#,
    );

    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "temporal failure should fail");
    let diagnostic = json_diagnostic(&output);
    assert_eq!(diagnostic["code"], "K0108");
    assert_contains(
        diagnostic["explanation"].as_str().unwrap_or_default(),
        "Temporal checks",
        "K0108 should explain trace-check evidence",
    );
}

#[test]
fn anti_overclaim_release_scan_stays_honest() {
    let root = workspace_root();
    let readme = std::fs::read_to_string(root.join("README.md")).expect("README should read");
    for forbidden in [
        "formally proves arbitrary",
        "proof of arbitrary Rust program correctness",
        "dashboard is required",
        "Strict changes runtime behavior",
        "replay external internals exactly",
    ] {
        assert!(
            !readme.contains(forbidden),
            "README should avoid overclaim `{forbidden}`"
        );
    }
    assert_contains(
        &readme,
        "Kobo can replay modeled wards and recorded boundaries",
        "README should state the bounded replay promise",
    );
}
