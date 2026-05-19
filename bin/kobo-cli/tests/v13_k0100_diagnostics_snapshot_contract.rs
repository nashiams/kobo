mod v09_common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use v09_common::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
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
fn k0101_liveness_escape_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0101-snapshot");
    let file = project.main_file(
        r#"
#[kobo::must_call(commit | rollback)]
struct Transaction { id: u64 }

fn external<T>(_value: T) {}

fn handler() {
    let tx = Transaction { id: 1 };
    external(tx);
}
"#,
    );

    let output = run_kobo(
        &[s("debt"), path_arg(&file), s("--liveness")],
        &project.root,
    );
    assert_failure_or_text(&output, "K0101");
    assert_contains(&output.combined(), "K0101", "escape should use K0101");
    assert_contains(
        &output.combined(),
        "Transaction",
        "K0101 snapshot should name the escaping type",
    );
    assert_contains(
        &output.combined(),
        "external call",
        "K0101 snapshot should name the escape boundary",
    );
}

#[test]
fn k0102_raw_nondeterminism_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0102-snapshot");
    let file = project.main_file(
        r#"
use std::time::SystemTime;

#[kobo::scenario(profile = "sync")]
fn raw_time() {
    let _now = SystemTime::now();
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
    assert_failure(&output, "raw nondeterminism should fail");
    let diagnostic = json_diagnostic(&output);
    assert_eq!(diagnostic["code"], "K0102");
    assert_eq!(diagnostic["category"], "nondeterminism");
    assert_contains(
        diagnostic["explanation"].as_str().unwrap_or_default(),
        "nondeterminism",
        "K0102 should explain deterministic replay impact",
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
fn k0104_replay_divergence_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0104-snapshot");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn stable_replay() {
    ward.task();
}
"#,
    );
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "stable replay witness should be recorded");
    let witness_path = first_witness_path(&project);
    let mut witness = read_witness(&witness_path);
    witness["events"][0]["kind"] = Value::String("mutated-event".to_owned());
    write_witness(&witness_path, &witness);

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(&replay, "mutated exact witness should diverge");
    let diagnostic = json_diagnostic(&replay);
    assert_eq!(diagnostic["code"], "K0104");
    assert_contains(
        &replay.combined(),
        "expected",
        "K0104 should include expected",
    );
    assert_contains(
        &replay.combined(),
        "observed",
        "K0104 should include observed",
    );
}

#[test]
fn k0105_budget_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0105-snapshot");
    let file = project.copy_fixture("sim/budget_overflow.kobo", "src/main.kobo");
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--event-budget"),
            s("8"),
            s("--error-format=json"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_failure(&output, "budget overflow should fail");
    let diagnostic = json_diagnostic(&output);
    assert_eq!(diagnostic["code"], "K0105");
    assert_contains(&diagnostic.to_string(), "8", "K0105 should include budget");
}

#[test]
fn k0106_shrink_safety_diagnostic_snapshot() {
    let project = TestProject::new("v13-k0106-snapshot");
    let file = project.main_file(
        r#"
#[kobo::scenario(profile = "async")]
fn shrink_replay() {
    ward.task();
}
"#,
    );
    let output = run_kobo(
        &[
            s("test"),
            s("--sim"),
            s("quick"),
            s("--witness-dir"),
            s(".kobo/witnesses"),
            path_arg(&file),
        ],
        &project.root,
    );
    assert_success(&output, "shrink replay witness should be recorded");
    let witness_path = first_witness_path(&project);
    let mut witness = read_witness(&witness_path);
    let event_count = witness["events"]
        .as_array()
        .map(|events| events.len())
        .unwrap_or_default();
    witness["shrink"] = json!({
        "replay_checked": false,
        "removed_event_ids": [],
        "original_event_count": event_count,
        "shrunk_event_count": event_count
    });
    write_witness(&witness_path, &witness);

    let replay = run_kobo(
        &[
            s("replay"),
            path_arg(&witness_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(&replay, "unsafe shrink metadata should fail");
    let diagnostic = json_diagnostic(&replay);
    assert_eq!(diagnostic["code"], "K0106");
    assert_contains(
        diagnostic["message"].as_str().unwrap_or_default(),
        "shrink",
        "K0106 should name unsafe shrink metadata",
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

fn first_witness_path(project: &TestProject) -> PathBuf {
    project
        .find_files_with_ext("kwit")
        .into_iter()
        .next()
        .expect("witness should exist")
}

fn read_witness(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("witness should read"))
        .expect("witness should parse")
}

fn write_witness(path: &Path, witness: &Value) {
    std::fs::write(
        path,
        serde_json::to_string_pretty(witness).expect("witness should serialize"),
    )
    .expect("witness should write");
}

fn assert_failure_or_text(output: &CliOutput, code: &str) {
    if output.status.success() {
        assert_contains(
            &output.combined(),
            code,
            "successful advisory command should still include requested diagnostic code",
        );
    } else {
        assert_failure(
            output,
            "diagnostic command should fail or emit advisory text",
        );
    }
}
