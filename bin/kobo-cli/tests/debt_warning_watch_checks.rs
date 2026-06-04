mod cli_test_support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cli_test_support::{
    assert_contains, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput, TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(60);

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
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn command_output(output: std::process::Output) -> CliOutput {
    CliOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn wait_child_output(mut child: Child, timeout: Duration) -> CliOutput {
    let started = Instant::now();
    loop {
        if child
            .try_wait()
            .expect("watch process status should be readable")
            .is_some()
        {
            return command_output(
                child
                    .wait_with_output()
                    .expect("watch process output should read"),
            );
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .expect("timed-out watch process output should read");
            panic!(
                "watch process did not exit before timeout\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(50));
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
    let project = TestProject::new("model-debt-watch-loop");
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
    assert_eq!(
        value["watch"]["reload_checkpoint"],
        "debt-precursor-snapshot"
    );
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
    let project = TestProject::new("model-watch-persist-reload");
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
    for expected in [
        "watcher evidence: metadata-only",
        "event batch: watch-batch-1",
        "debounce window: watch-window-1 (200ms)",
        "restart decision: rerun",
        "child lifecycle: no child process started",
    ] {
        assert_contains(
            &text,
            expected,
            "watch plan should expose typed watcher and supervisor evidence",
        );
    }
}

#[test]
fn watch_simple_observes_module_change_and_persists_reload_state() {
    let project = TestProject::new("model-watch-real-reload");
    let main = project.main_file("mod service;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--simple")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&service, "fn helper() { let value = 1; }\n")
        .expect("watched module should be writable");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --simple should exit after one scoped module change",
    );
    let text = output.combined();
    for expected in [
        "scoped-persist-reload",
        "persisted state loaded",
        "changed file: src/service.kobo",
        "event batch: watch-batch-1",
        "debounce window: watch-window-1",
        "restart decision: rerun",
        "child lifecycle: in-process rerun finished",
        "reload checkpoint: source-map-and-diagnostics",
        "restartable: true",
    ] {
        assert_contains(
            &text,
            expected,
            "watch output should expose real scoped reload evidence",
        );
    }

    let state_path = project.root.join(".kobo/watch/source-watch.json");
    assert!(
        state_path.is_file(),
        "watch should persist restartable source state"
    );
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(state["reload_checkpoint"], "source-map-and-diagnostics");
    assert_eq!(state["restartable"], Value::Bool(true));
    assert_eq!(state["watcher_evidence"], "metadata-only");
    assert_contains(
        &state["scope"]["files"].to_string(),
        "src/service.kobo",
        "watch state should retain scoped module files",
    );
    assert_eq!(state["changes"][0]["path"], "src/service.kobo");
    assert_eq!(state["changes"][0]["event_kind"], "modify");
    assert_eq!(state["event_batches"][0]["id"], "watch-batch-1");
    assert_eq!(state["event_batches"][0]["replay_grade"], "partial");
    assert_eq!(
        state["event_batches"][0]["events"][0]["paths"][0]["role"],
        "source_path"
    );
    assert_eq!(state["debounce_windows"][0]["id"], "watch-window-1");
    assert_eq!(
        state["debounce_windows"][0]["interval_ms"],
        Value::from(200)
    );
    assert_eq!(state["restart_decisions"][0]["action"], "rerun");
    assert_eq!(
        state["child_lifecycle_obligations"][0]["resolution"],
        "in_process_rerun_finished"
    );
}

#[test]
fn watch_simple_batches_multiple_module_changes_into_one_restart_window() {
    let project = TestProject::new("model-watch-batched-reload");
    let main = project.main_file("mod service;\nmod worker;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");
    let worker = project.write("src/worker.kobo", "fn helper() {}\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--simple")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&service, "fn helper() { let value = 1; }\n")
        .expect("watched service module should be writable");
    std::fs::write(&worker, "fn helper() { let value = 2; }\n")
        .expect("watched worker module should be writable");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --simple should exit after one batched scoped module change",
    );
    let text = output.combined();
    assert_eq!(
        text.matches("Triggering rebuild").count(),
        1,
        "same-window module changes should cause one restart:\n{text}",
    );

    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(
        state["event_batches"][0]["events"].as_array().map(Vec::len),
        Some(2),
        "batched state should retain both changed source files",
    );
    let events = state["event_batches"][0]["events"].to_string();
    assert_contains(
        &events,
        "src/service.kobo",
        "batched event evidence should include service module",
    );
    assert_contains(
        &events,
        "src/worker.kobo",
        "batched event evidence should include worker module",
    );
    assert_eq!(
        state["restart_decisions"].as_array().map(Vec::len),
        Some(1),
        "batched changes should produce one restart decision",
    );
}

#[test]
fn watch_simple_records_failed_rerun_outcome_after_execution() {
    let project = TestProject::new("model-watch-failed-rerun");
    let main = project.main_file("fn main() {}\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--simple")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&main, "fn main() {\n    let broken = ;\n}\n")
        .expect("watched main module should be writable");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --simple should keep running semantics even when the rerun reports diagnostics",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(state["restart_decisions"][0]["outcome"], "failed");
    assert!(
        state["restart_decisions"][0]["diagnostic_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "failed rerun should persist diagnostic count",
    );
    assert_eq!(
        state["child_lifecycle_obligations"][0]["resolution"],
        "in_process_rerun_finished"
    );
}

#[test]
fn watch_simple_discovers_new_kobo_file_after_start() {
    let project = TestProject::new("model-watch-create-event");
    let main = project.main_file("fn main() {}\n");
    let created = project.root.join("src/new_module.kobo");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--simple")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&created, "fn helper() {}\n").expect("new watched module should write");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --simple should exit after a new Kobo source appears",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(state["changes"][0]["path"], "src/new_module.kobo");
    assert_eq!(state["changes"][0]["event_kind"], "create");
    assert_eq!(state["event_batches"][0]["events"][0]["kind"], "create");
}

#[test]
fn watch_simple_records_removed_kobo_file_instead_of_dropping_it() {
    let project = TestProject::new("model-watch-remove-event");
    let main = project.main_file("mod service;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--simple")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::remove_file(&service).expect("watched service module should be removable");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --simple should exit after a watched Kobo source is removed",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(state["changes"][0]["path"], "src/service.kobo");
    assert_eq!(state["changes"][0]["event_kind"], "remove");
    assert_eq!(state["event_batches"][0]["events"][0]["kind"], "remove");
    assert_eq!(state["event_batches"][0]["replay_grade"], "partial");
}
