mod v09_common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use v09_common::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};

const V13_TIMEOUT: Duration = Duration::from_secs(60);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root should exist")
}

fn fixture(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, V13_TIMEOUT)
}

fn command_output(output: std::process::Output) -> CliOutput {
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn debt_outputs_tiered_summary_migration_estimate_json_and_summary() {
    let root = workspace_root();
    let file = fixture("tests/fixtures/debt_report_full.kobo");
    let json = run_kobo(&[s("debt"), path_arg(&file), s("--json")], root.as_path());
    assert_success(&json, "debt --json should succeed");
    let value: Value = serde_json::from_str(&json.stdout).expect("debt JSON should parse");
    assert_eq!(value["schema_version"], 1);
    assert!(value["complexity_breakdown"]["tier1"].is_number());
    assert!(value["tiered_summary"]["tier1"].is_number());
    assert_contains(
        &value["migration_estimate"].to_string(),
        "estimated",
        "debt JSON should include migration estimate evidence",
    );

    let summary = run_kobo(
        &[s("debt"), path_arg(&file), s("--summary")],
        root.as_path(),
    );
    assert_success(&summary, "debt --summary should succeed");
    assert_eq!(
        summary.stdout.lines().count(),
        1,
        "summary should stay one line"
    );
    for expected in ["T1:", "T2:", "T3:", "migration estimate"] {
        assert_contains(
            &summary.stdout,
            expected,
            "summary should expose tiered estimate",
        );
    }
}

#[test]
fn debt_watch_mode_reports_precursor_warning_changes() {
    let root = workspace_root();
    let file = fixture("tests/ui/K0080_P1_node.kobo");
    let output = run_kobo(
        &[s("debt"), path_arg(&file), s("--watch"), s("--summary")],
        root.as_path(),
    );
    assert_success(&output, "debt --watch --summary should be bounded");
    let text = output.combined();
    for expected in [
        "debt watch",
        "scoped-persist-reload",
        "actual scan observations=1",
        "precursor",
        "K0080-P1",
        "rerun target: kobo debt",
    ] {
        assert_contains(
            &text,
            expected,
            "debt watch should report precursor changes",
        );
    }

    let json = run_kobo(
        &[s("debt"), path_arg(&file), s("--watch"), s("--json")],
        root.as_path(),
    );
    assert_success(&json, "debt --watch --json should report scan observations");
    let value: Value = serde_json::from_str(&json.stdout).expect("watch JSON should parse");
    assert_eq!(value["watch"]["actual_scan"], Value::Bool(true));
    assert_eq!(value["observations"][0]["kind"], "initial_scan");
    assert_contains(
        &value["observations"][0].to_string(),
        "K0080-P1",
        "watch JSON should carry actual precursor observation data",
    );
}

#[test]
fn debt_watch_persists_reload_state_across_scan_ticks() {
    let project = TestProject::new("v13-debt-watch-loop");
    let file = project.main_file(
        r#"
struct GraphNode {
    left: Rc<RefCell<GraphNode>>,
    right: Rc<RefCell<GraphNode>>,
    data: i32,
}
"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("debt")
        .arg(&file)
        .arg("--watch")
        .arg("--json")
        .env("KOBO_DEBT_WATCH_TICKS", "2")
        .current_dir(&project.root)
        .output()
        .expect("debt watch should launch");
    let output = command_output(output);
    assert_success(&output, "debt --watch should run bounded scan ticks");
    let value: Value = serde_json::from_str(&output.stdout).expect("watch JSON should parse");
    assert_eq!(value["watch"]["actual_scan"], Value::Bool(true));
    assert_eq!(value["watch"]["observation_count"], Value::from(2));
    assert_eq!(value["watch"]["reload_checkpoint"], "debt-precursor-snapshot");
    assert_eq!(value["watch"]["persisted_state_loaded"], Value::Bool(true));
    assert!(
        project.root.join(".kobo/watch/debt-watch.json").is_file(),
        "debt watch should persist restartable state in the project .kobo directory",
    );
    assert_eq!(
        value["observations"].as_array().map(Vec::len),
        Some(2),
        "watch JSON should retain every bounded scan observation",
    );
}

#[test]
fn k0080_precursor_patterns_are_integrated() {
    let root = workspace_root();
    let file = fixture("tests/ui/K0080_P1_node.kobo");
    let check = run_kobo(&[s("check"), path_arg(&file)], root.as_path());
    assert_success(&check, "K0080-P precursor check should be advisory");
    assert_contains(
        &check.combined(),
        "note[K0080-P1]",
        "check should surface the precursor note",
    );

    let debt = run_kobo(&[s("debt"), path_arg(&file), s("--json")], root.as_path());
    assert_success(&debt, "debt JSON should include K0080-P precursors");
    assert_contains(
        &debt.stdout,
        "K0080-P1",
        "debt JSON should integrate the same precursor pattern",
    );
}

#[test]
fn watch_persist_reload_loop_is_scoped_and_restartable() {
    let project = TestProject::new("v13-watch-persist-reload");
    let main = project.main_file("mod service;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");
    let output = run_kobo(
        &[
            s("watch"),
            s("--plan"),
            s("--changed"),
            path_arg(&service),
            path_arg(&main),
        ],
        &project.root,
    );
    assert_success(&output, "watch --plan should be bounded");
    let text = output.combined();
    for expected in [
        "scoped-persist-reload",
        "persisted scope",
        "reload checkpoint",
        "restartable",
        "src/service.kobo",
        "rerun target: kobo check",
    ] {
        assert_contains(
            &text,
            expected,
            "watch plan should expose scoped restart state",
        );
    }
}
