mod cli_test_support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
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

fn release_watch_state_with_adapters(replay_grade: &str, event_evidence_grade: &str) -> String {
    serde_json::json!({
        "schema_version": 1,
        "mode": "source_watch_state",
        "watcher_evidence": "metadata-only",
        "event_batches": [
            {
                "replay_grade": replay_grade,
                "events": [
                    {
                        "evidence_grade": event_evidence_grade
                    }
                ]
            }
        ],
        "debounce_windows": [
            {
                "timer_evidence": "runtime-observed",
                "replay_grade": "partial"
            }
        ],
        "child_lifecycle_obligations": [
            {
                "command_kind": "in_process_check",
                "resolution": "in_process_rerun_finished"
            }
        ],
        "adapter_summaries": [
            {
                "kind": "path_filter",
                "replay_confidence": "modelled",
                "conformance_tests": ["source-watch-path-match"]
            },
            {
                "kind": "async_runtime",
                "replay_confidence": "partial",
                "scheduler_facts": ["source-watch-task-order"],
                "conformance_tests": ["source-watch-task-order"]
            }
        ],
        "external_comparisons": [
            {
                "implementation": "source-watch",
                "behavior": "path filtering and async ordering",
                "disposition": "formal_adapter_contract"
            }
        ]
    })
    .to_string()
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

fn wait_for_watch_output(mut child: Child, timeout: Duration) -> CliOutput {
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
            return command_output(output);
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
fn bare_debt_commands_use_project_default_source() {
    let project = TestProject::new("model-bare-debt-default-source");
    project.main_file("fn main() {}\n");

    let summary = run_kobo(&[s("debt"), s("--summary")], &project.root);
    assert_success(
        &summary,
        "bare debt --summary should use the project default source",
    );
    assert_eq!(
        summary.stdout.lines().count(),
        1,
        "bare debt summary should stay copy-paste friendly",
    );
    assert_contains(
        &summary.stdout,
        "migration estimate",
        "bare debt summary should run the same report as file-scoped debt",
    );

    let json = run_kobo(&[s("debt"), s("--json")], &project.root);
    assert_success(
        &json,
        "bare debt --json should use the project default source",
    );
    let value: Value = serde_json::from_str(&json.stdout).expect("bare debt JSON should parse");
    assert_eq!(value["schema_version"], 1);

    let watch = run_kobo(&[s("debt"), s("--watch"), s("--summary")], &project.root);
    assert_success(
        &watch,
        "bare debt --watch should use the project default source",
    );
    assert_contains(
        &watch.stdout,
        "debt watch scoped-persist-reload",
        "bare debt watch should keep the scoped watch evidence shape",
    );

    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        r#"{
  "schema_version": 1,
  "mode": "source_watch_state",
  "scope": {
    "root": "src/main.kobo",
    "files": ["src/main.kobo"]
  },
  "watcher_evidence": "metadata-only",
  "adapter_summaries": [
    {"kind": "path_filter", "replay_confidence": "modelled", "conformance_tests": ["source-watch-path-match"]},
    {"kind": "async_runtime", "replay_confidence": "partial", "scheduler_facts": ["source-watch-task-order"], "conformance_tests": ["source-watch-task-order"]}
  ],
  "event_batches": [
    {
      "replay_grade": "partial",
      "events": [
        {
          "evidence_grade": "metadata_only"
        }
      ]
    }
  ],
  "debounce_windows": [
    {
      "timer_evidence": "metadata-only",
      "replay_grade": "partial"
    }
  ],
  "child_lifecycle_obligations": [
    {
      "command_kind": "in_process_check",
      "resolution": "in_process_rerun_finished"
    }
  ]
}"#,
    )
    .expect("watch state should write");

    let adapter_summary = run_kobo(&[s("debt"), s("--summary")], &project.root);
    assert_success(
        &adapter_summary,
        "debt --summary should include adapter debt for persisted watch evidence",
    );
    assert_contains(
        &adapter_summary.stdout,
        "adapter debt:",
        "summary should include adapter debt by subsystem",
    );
    assert_contains(
        &adapter_summary.stdout,
        "watcher=acceptable",
        "watcher metadata-only evidence should be visible as acceptable adapter debt",
    );
    assert_contains(
        &adapter_summary.stdout,
        "process=acceptable",
        "in-process supervisor evidence should be visible by subsystem",
    );
    assert_contains(
        &adapter_summary.stdout,
        "time=acceptable",
        "debounce timer evidence should be visible by subsystem",
    );
    assert_contains(
        &adapter_summary.stdout,
        "path_filter=acceptable",
        "path filter evidence should be visible by subsystem",
    );
    assert_contains(
        &adapter_summary.stdout,
        "async_runtime=acceptable",
        "async runtime evidence should be visible by subsystem",
    );
}

#[test]
fn debt_report_explains_kobo_rust_and_boundary_modules() {
    let project = TestProject::new("debt-module-ownership-report");
    project.main_file("fn main() {}\n");
    project.write("src/adapter.rs", "pub fn run_adapter() {}\n");
    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        r#"{
  "schema_version": 1,
  "mode": "source_watch_state",
  "watcher_evidence": "metadata-only",
  "event_batches": [
    {
      "replay_grade": "partial",
      "events": [
        {
          "evidence_grade": "metadata_only"
        }
      ]
    }
  ],
  "debounce_windows": [
    {
      "timer_evidence": "metadata-only",
      "replay_grade": "partial"
    }
  ],
  "child_lifecycle_obligations": [
    {
      "command_kind": "in_process_check",
      "resolution": "in_process_rerun_finished"
    }
  ]
}"#,
    )
    .expect("watch state should write");

    let summary = run_kobo(&[s("debt"), s("--summary")], &project.root);
    assert_success(&summary, "debt summary should explain module ownership");
    for expected in [
        "modules:",
        "kobo-owned=1",
        "rust-owned=1",
        "boundary-debt=5",
    ] {
        assert_contains(
            &summary.stdout,
            expected,
            "summary should explain Kobo/Rust/boundary module ownership",
        );
    }

    let json = run_kobo(&[s("debt"), s("--json")], &project.root);
    assert_success(&json, "debt JSON should explain module ownership");
    let value: Value = serde_json::from_str(&json.stdout).expect("debt JSON should parse");
    assert_eq!(
        value["module_ownership"]["kobo_owned"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        value["module_ownership"]["rust_owned"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        value["module_ownership"]["boundary_debt"]
            .as_array()
            .map(Vec::len),
        Some(5)
    );
}

#[test]
fn release_profile_blocks_unresolved_watch_lifecycle_debt() {
    let project = TestProject::new("release-watch-lifecycle-gate");
    let main = project.main_file("fn main() {}\n");
    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        r#"{
  "schema_version": 1,
  "mode": "source_watch_state",
  "watcher_evidence": "metadata-only",
  "event_batches": [
    {
      "replay_grade": "partial",
      "events": [
        {
          "evidence_grade": "metadata_only"
        }
      ]
    }
  ],
  "debounce_windows": [
    {
      "timer_evidence": "metadata-only",
      "replay_grade": "partial"
    }
  ],
  "child_lifecycle_obligations": [
    {
      "command_kind": "in_process_check",
      "resolution": "unresolved_started_child"
    }
  ]
}"#,
    )
    .expect("watch state should write");

    let check = run_kobo(
        &[s("check"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &check,
        "release check should block unresolved watch lifecycle debt",
    );
    assert_contains(
        &check.combined(),
        "release watch lifecycle gate",
        "release check should name the watch lifecycle gate",
    );

    let build = run_kobo(
        &[s("build"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &build,
        "release build should block unresolved watch lifecycle debt",
    );
    assert_contains(
        &build.combined(),
        "release watch lifecycle gate",
        "release build should name the watch lifecycle gate",
    );
}

#[test]
fn release_profile_blocks_incomplete_watch_debounce_debt() {
    let project = TestProject::new("release-watch-debounce-gate");
    let main = project.main_file("fn main() {}\n");
    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        r#"{
  "schema_version": 1,
  "mode": "source_watch_state",
  "watcher_evidence": "metadata-only",
  "event_batches": [
    {
      "replay_grade": "partial",
      "events": [
        {
          "evidence_grade": "metadata_only"
        }
      ]
    }
  ],
  "debounce_windows": [
    {
      "timer_evidence": "metadata-only",
      "replay_grade": "debt"
    }
  ],
  "child_lifecycle_obligations": [
    {
      "command_kind": "in_process_check",
      "resolution": "in_process_rerun_finished"
    }
  ]
}"#,
    )
    .expect("watch state should write");

    let check = run_kobo(
        &[s("check"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &check,
        "release check should block incomplete watch debounce debt",
    );
    assert_contains(
        &check.combined(),
        "incomplete debounce shutdown evidence",
        "release check should name debounce debt",
    );

    let build = run_kobo(
        &[s("build"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &build,
        "release build should block incomplete watch debounce debt",
    );
    assert_contains(
        &build.combined(),
        "incomplete debounce shutdown evidence",
        "release build should name debounce debt",
    );
}

#[test]
fn release_profile_blocks_missing_watch_adapter_debt() {
    let project = TestProject::new("release-watch-adapter-gate");
    let main = project.main_file("fn main() {}\n");
    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        r#"{
  "schema_version": 1,
  "mode": "source_watch_state",
  "watcher_evidence": "metadata-only",
  "event_batches": [
    {
      "replay_grade": "partial",
      "events": [
        {
          "evidence_grade": "metadata_only"
        }
      ]
    }
  ],
  "debounce_windows": [
    {
      "timer_evidence": "runtime-observed",
      "replay_grade": "partial"
    }
  ],
  "child_lifecycle_obligations": [
    {
      "command_kind": "in_process_check",
      "resolution": "in_process_rerun_finished"
    }
  ]
}"#,
    )
    .expect("watch state should write");

    let check = run_kobo(
        &[s("check"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &check,
        "release check should block missing watch adapter debt",
    );
    assert_contains(
        &check.combined(),
        "missing path filter adapter evidence",
        "release check should name missing path filter adapter evidence",
    );

    let build = run_kobo(
        &[s("build"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &build,
        "release build should block missing watch adapter debt",
    );
    assert_contains(
        &build.combined(),
        "missing path filter adapter evidence",
        "release build should name missing path filter adapter evidence",
    );
}

#[test]
fn release_profile_blocks_watch_replay_debt() {
    let project = TestProject::new("release-watch-replay-gate");
    let main = project.main_file("fn main() {}\n");
    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        release_watch_state_with_adapters("debt", "metadata_only"),
    )
    .expect("watch state should write");

    let check = run_kobo(
        &[s("check"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(&check, "release check should block watch replay debt");
    assert_contains(
        &check.combined(),
        "incomplete watch replay evidence",
        "release check should name replay debt",
    );

    let build = run_kobo(
        &[s("build"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(&build, "release build should block watch replay debt");
    assert_contains(
        &build.combined(),
        "incomplete watch replay evidence",
        "release build should name replay debt",
    );
}

#[test]
fn release_profile_blocks_unknown_platform_event_evidence() {
    let project = TestProject::new("release-watch-platform-gate");
    let main = project.main_file("fn main() {}\n");
    let watch_dir = project.root.join(".kobo/watch");
    std::fs::create_dir_all(&watch_dir).expect("watch state directory should create");
    std::fs::write(
        watch_dir.join("source-watch.json"),
        release_watch_state_with_adapters("partial", "unknown"),
    )
    .expect("watch state should write");

    let check = run_kobo(
        &[s("check"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &check,
        "release check should block unknown platform event evidence",
    );
    assert_contains(
        &check.combined(),
        "unknown watch platform event evidence",
        "release check should name platform event evidence debt",
    );

    let build = run_kobo(
        &[s("build"), s("--profile"), s("release"), path_arg(&main)],
        &project.root,
    );
    assert_failure(
        &build,
        "release build should block unknown platform event evidence",
    );
    assert_contains(
        &build.combined(),
        "unknown watch platform event evidence",
        "release build should name platform event evidence debt",
    );
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
    let state_text = state.to_string();
    for expected in [
        "adapter_summaries",
        "path_filter",
        "async_runtime",
        "external_comparisons",
        "source-watch-task-order",
        "source-watch-path-match",
    ] {
        assert_contains(
            &state_text,
            expected,
            "persisted watch state should carry the same formal adapter summaries used by replay",
        );
    }
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
fn watch_simple_records_coalesced_duplicate_changes_in_one_window() {
    let project = TestProject::new("model-watch-duplicate-coalesce");
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
        .expect("first service edit should write");
    thread::sleep(Duration::from_millis(250));
    std::fs::write(&service, "fn helper() { let value = 2; }\n")
        .expect("second service edit should write");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --simple should exit after coalesced duplicate changes",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(
        state["event_batches"][0]["events"][0]["duplicate_or_coalesced"],
        "coalesced"
    );
    assert_eq!(
        state["debounce_windows"][0]["timer_cancelled"],
        Value::Bool(true),
        "coalesced duplicate change should record debounce extension",
    );
    assert_contains(
        &state["debounce_windows"][0]["extension_cause"].to_string(),
        "src/service.kobo",
        "debounce window should explain which event extended the timer",
    );
}

#[test]
fn watch_simple_two_debounce_windows_produce_two_restart_decisions() {
    let project = TestProject::new("model-watch-two-windows");
    let main = project.main_file("mod service;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--simple")
        .arg(&main)
        .env("KOBO_WATCH_MAX_WINDOWS", "2")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&service, "fn helper() { let value = 1; }\n")
        .expect("first service edit should write");
    thread::sleep(Duration::from_millis(700));
    std::fs::write(&service, "fn helper() { let value = 2; }\n")
        .expect("second service edit should write");

    let output = wait_for_watch_output(child, Duration::from_secs(10));
    assert_success(&output, "watch should exit after two windows");
    let text = output.combined();
    assert_eq!(
        text.matches("Triggering rebuild").count(),
        2,
        "two debounce windows should cause two reruns:\n{text}",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(
        state["restart_decisions"].as_array().map(Vec::len),
        Some(2),
        "persisted watch state should retain both restart decisions",
    );
    assert_eq!(state["event_batches"][1]["id"], "watch-batch-2");
    assert_eq!(state["debounce_windows"][1]["id"], "watch-window-2");
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

#[test]
fn watch_simple_marks_rename_as_metadata_candidate_with_raw_events() {
    let project = TestProject::new("model-watch-rename-event");
    let main = project.main_file("mod service;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");
    let renamed = project.root.join("src/moved_service.kobo");

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
    std::fs::rename(&service, &renamed).expect("watched service module should rename");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(&output, "watch should exit after one rename window");
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(
        state["event_batches"][0]["events"][0]["kind"],
        "rename_candidate"
    );
    assert_eq!(
        state["event_batches"][0]["events"][0]["evidence_grade"],
        "modelled_from_metadata"
    );
    let paths = state["event_batches"][0]["events"][0]["paths"].to_string();
    for expected in [
        "source_path",
        "destination_path",
        "parent_path",
        "src/service.kobo",
        "src/moved_service.kobo",
    ] {
        assert_contains(&paths, expected, "rename evidence should keep path roles");
    }
    assert_contains(
        &state["event_batches"][0]["known_path_roles"].to_string(),
        "destination_path",
        "batch should publish supported path roles",
    );
    let raw_events = state["event_batches"][0]["events"][0]["raw_events"].to_string();
    assert_contains(
        &raw_events,
        "remove",
        "rename candidate should preserve the raw remove event",
    );
    assert_contains(
        &raw_events,
        "create",
        "rename candidate should preserve the raw create event",
    );
    assert_contains(
        &state["changes"][0].to_string(),
        "modelled_from_metadata",
        "compact change list should also expose the modelled evidence grade",
    );
}

#[test]
fn watch_simple_keeps_multiple_create_remove_pairs_as_raw_events() {
    let project = TestProject::new("model-watch-multi-remove-create");
    let main = project.main_file("mod service;\nmod extra;\nfn main() {}\n");
    let service = project.write("src/service.kobo", "fn helper() {}\n");
    let extra = project.write("src/extra.kobo", "fn extra() {}\n");

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
    std::fs::remove_file(&service).expect("first watched source should remove");
    std::fs::remove_file(&extra).expect("second watched source should remove");
    project.write("src/new_service.kobo", "fn helper() {}\n");
    project.write("src/new_extra.kobo", "fn extra() {}\n");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch should exit after one multi-pair remove/create window",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    let events = state["event_batches"][0]["events"]
        .as_array()
        .expect("watch events should be an array");
    let event_kinds = events
        .iter()
        .map(|event| event["kind"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();

    assert!(
        !event_kinds.contains(&"rename_candidate"),
        "ambiguous multi-pair remove/create windows should not invent one rename candidate: {event_kinds:?}",
    );
    assert_eq!(
        event_kinds.iter().filter(|kind| **kind == "remove").count(),
        2,
        "both removes should remain visible as raw watcher facts",
    );
    assert_eq!(
        event_kinds.iter().filter(|kind| **kind == "create").count(),
        2,
        "both creates should remain visible as raw watcher facts",
    );
}

#[test]
fn watch_simple_records_metadata_only_change() {
    let project = TestProject::new("model-watch-metadata-event");
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
    let mut permissions = std::fs::metadata(&service)
        .expect("service metadata should read")
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&service, permissions).expect("service permissions should update");

    let output = wait_child_output(child, Duration::from_secs(8));
    let mut cleanup_permissions = std::fs::metadata(&service)
        .expect("service metadata should read for cleanup")
        .permissions();
    cleanup_permissions.set_readonly(false);
    let _ = std::fs::set_permissions(&service, cleanup_permissions);
    assert_success(&output, "watch should exit after metadata-only change");
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(state["changes"][0]["event_kind"], "metadata");
    assert_eq!(state["event_batches"][0]["events"][0]["kind"], "metadata");
}

#[test]
fn watch_build_persists_codegen_supervisor_evidence() {
    let project = TestProject::new("model-watch-build-evidence");
    let main = project.main_file("fn main() { println!(\"ok\"); }\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--build")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch build process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&main, "fn main() { println!(\"updated\"); }\n")
        .expect("watched main file should update");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(&output, "watch --build should exit after one build rerun");
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(
        state["child_lifecycle_obligations"][0]["command_kind"],
        "in_process_codegen"
    );
    assert_eq!(state["restart_decisions"][0]["outcome"], "succeeded");
    assert_eq!(
        state["child_lifecycle_obligations"][0]["resolution"],
        "in_process_rerun_finished"
    );
}

#[test]
fn watch_build_persists_failed_codegen_supervisor_evidence() {
    let project = TestProject::new("model-watch-build-failed-evidence");
    let main = project.main_file("fn main() { println!(\"ok\"); }\n");

    let child = Command::new(env!("CARGO_BIN_EXE_kobo"))
        .arg("watch")
        .arg("--build")
        .arg(&main)
        .env("KOBO_WATCH_ONCE", "1")
        .current_dir(&project.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("watch build process should launch");

    thread::sleep(Duration::from_millis(500));
    std::fs::write(&main, "fn main() {\n    let broken = ;\n}\n")
        .expect("watched main file should update to invalid source");

    let output = wait_child_output(child, Duration::from_secs(8));
    assert_success(
        &output,
        "watch --build should keep watch process semantics when codegen fails",
    );
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state: Value = serde_json::from_str(
        &std::fs::read_to_string(&state_path).expect("watch state should read"),
    )
    .expect("watch state should parse");
    assert_eq!(
        state["child_lifecycle_obligations"][0]["command_kind"],
        "in_process_codegen"
    );
    assert_eq!(state["restart_decisions"][0]["outcome"], "failed");
    assert!(
        state["restart_decisions"][0]["diagnostic_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "failed build rerun should persist diagnostic count",
    );
    assert_eq!(
        state["child_lifecycle_obligations"][0]["resolution"],
        "in_process_rerun_finished"
    );
}
