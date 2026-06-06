mod cli_test_support;

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s, CliOutput,
    TestProject,
};
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(20);

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

fn wait_child_output(mut child: std::process::Child, timeout: Duration) -> CliOutput {
    let started = std::time::Instant::now();
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

fn copy_watchexec_trace(project: &TestProject) -> std::path::PathBuf {
    let fixture = TestProject::repo_root()
        .join("tests")
        .join("fixtures")
        .join("watchexec_supervisor_policy")
        .join("save_restart_trace.json");
    let contents = fs::read_to_string(&fixture).unwrap_or_else(|error| {
        panic!(
            "fixture `{}` should be readable: {error}",
            fixture.display()
        )
    });
    project.write("traces/save_restart.json", &contents)
}

fn trace_with_events(events: &str) -> String {
    format!(
        r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": [
    {{
      "kind": "watcher",
      "name": "notify-like",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["raw_event"],
      "modeled_facts": ["event_kind", "paths", "ordering", "duplicates"],
      "unsupported_guarantees": ["global_total_order"],
      "replay_confidence": "metadata-only",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    }},
    {{
      "kind": "process",
      "name": "process-supervisor",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["spawn", "exit", "signal", "kill", "detach"],
      "modeled_facts": ["child_id", "policy", "exit_code", "resolution"],
      "unsupported_guarantees": ["platform_signal_equivalence"],
      "replay_confidence": "modelled",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    }},
    {{
      "kind": "time",
      "name": "logical-debounce",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["timer_create", "timer_cancel", "timer_fire"],
      "modeled_facts": ["window", "membership", "fire_order"],
      "unsupported_guarantees": ["real_os_time_determinism"],
      "replay_confidence": "partial",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    }}
  ],
  "external_comparisons": [
    {{
      "implementation": "watchexec",
      "behavior": "coalesced filesystem events",
      "disposition": "replay_fixture",
      "reason": "normalized debounce batches preserve duplicate raw events",
      "evidence_anchor": "events[]"
    }}
  ],
  "events": {events}
}}"#
    )
}

fn valid_watchexec_events() -> &'static str {
    r#"[
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 1,
      "timestamp_ms": 10,
      "duplicate_marker": "unique",
      "evidence_grade": "metadata_only"
    },
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 1,
      "timestamp_ms": 11,
      "duplicate_marker": "duplicate",
      "evidence_grade": "metadata_only"
    },
    {
      "kind": "timer_fired",
      "window": 1,
      "timestamp_ms": 210
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "timestamp_ms": 211
    },
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 212
    },
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 2,
      "timestamp_ms": 250,
      "duplicate_marker": "unique",
      "evidence_grade": "metadata_only"
    },
    {
      "kind": "child_exit",
      "child_id": "cmd-1",
      "exit_code": 0,
      "timestamp_ms": 260
    },
    {
      "kind": "timer_fired",
      "window": 2,
      "timestamp_ms": 450
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "timestamp_ms": 451
    },
    {
      "kind": "child_start",
      "child_id": "cmd-2",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 452
    },
    {
      "kind": "child_exit",
      "child_id": "cmd-2",
      "exit_code": 0,
      "timestamp_ms": 500
    }
  ]"#
}

#[test]
fn live_watch_state_imports_to_replayable_trace_witness() {
    let project = TestProject::new("watchexec-live-watch-state");
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
    fs::write(&service, "fn helper() { let value = 1; }\n")
        .expect("watched service module should be writable");
    let watch = wait_child_output(child, Duration::from_secs(8));
    assert_success(&watch, "live watch should produce source watch state");

    let state_path = project.root.join(".kobo/watch/source-watch.json");
    assert!(
        state_path.is_file(),
        "live watch should persist source-watch.json"
    );
    let witness = project.root.join(".kobo/witnesses/live-watch.kwit");
    let import = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&state_path),
            s("--witness-out"),
            path_arg(&witness),
        ],
        &project.root,
    );
    assert_success(
        &import,
        "live source watch state should import into a replayable witness",
    );
    let witness_json: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("live watch witness should read"),
    )
    .expect("live watch witness should parse");
    assert_eq!(witness_json["source"]["kind"], "source_watch_state");
    assert_contains(
        &witness_json["normalized"]["event_batches"].to_string(),
        "src/service.kobo",
        "live watch witness should preserve observed changed path",
    );
    assert_contains(
        &witness_json["external_comparisons"].to_string(),
        "watchfiles",
        "live watch witness should carry external comparison evidence",
    );
    assert_contains(
        &witness_json["external_comparisons"].to_string(),
        "go-air",
        "live watch witness should not drop selected external boundary classifications",
    );

    let replay = run_kobo(&[s("replay"), path_arg(&witness)], &project.root);
    assert_success(&replay, "live watch witness should replay");
}

#[test]
fn watchexec_policy_slice_checks_and_exports_as_mixed_cargo_project() {
    let project = TestProject::new("watchexec-policy-slice");
    let fixture = TestProject::repo_root()
        .join("tests")
        .join("fixtures")
        .join("watchexec_supervisor_policy")
        .join("supervisor_policy.kobo");
    let source = fs::read_to_string(&fixture).unwrap_or_else(|error| {
        panic!(
            "fixture `{}` should be readable: {error}",
            fixture.display()
        )
    });
    let policy = project.write("src/supervisor_policy.kobo", &source);

    let check = run_kobo(&[s("check"), path_arg(&policy)], &project.root);
    assert_success(&check, "Kobo-owned supervisor policy slice should check");

    let cargo_dir = project.root.join("target/policy-clean");
    let inspect = run_kobo(
        &[
            s("inspect"),
            s("--clean"),
            s("--cargo"),
            path_arg(&cargo_dir),
            path_arg(&policy),
        ],
        &project.root,
    );
    assert_success(
        &inspect,
        "Kobo-owned policy slice should export a mixed Cargo project",
    );
    let generated = fs::read_to_string(cargo_dir.join("src/main.rs"))
        .expect("generated Rust policy slice should read");
    for expected in [
        "RawEvent",
        "RestartPolicy",
        "SupervisorState",
        "rust_notify_adapter_emit",
        "rust_process_adapter_apply",
    ] {
        assert_contains(
            &generated,
            expected,
            "generated Rust should preserve reviewable policy and adapter names",
        );
    }
}

#[test]
fn watch_trace_import_emits_replayable_witness_with_adapter_contracts() {
    let project = TestProject::new("watchexec-trace-import");
    let trace = copy_watchexec_trace(&project);
    let witness = project.root.join(".kobo/witnesses/watchexec-policy.kwit");

    let output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&trace),
            s("--witness-out"),
            path_arg(&witness),
        ],
        &project.root,
    );
    assert_success(&output, "watch trace import should succeed");
    assert_contains(
        &output.combined(),
        "watch trace witness",
        "trace import should report the generated witness",
    );

    let witness_json: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("trace witness should be readable"),
    )
    .expect("trace witness should parse");
    assert_eq!(witness_json["mode"], "watch_trace_witness");
    assert_eq!(witness_json["replay_grade"], "partial");
    assert_eq!(
        witness_json["adapter_summaries"].as_array().map(Vec::len),
        Some(3)
    );
    assert_contains(
        &witness_json["external_comparisons"].to_string(),
        "chokidar",
        "trace witness should preserve selected external comparison evidence",
    );
    assert_contains(
        &witness_json["external_comparisons"].to_string(),
        "atomic write",
        "comparison evidence should name mature watcher behavior",
    );
    assert_eq!(
        witness_json["normalized"]["event_batches"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "two debounce windows should stay distinct"
    );
    assert_eq!(
        witness_json["normalized"]["event_batches"][0]["events"][0]["duplicate_or_coalesced"],
        "coalesced",
        "duplicate raw watcher events should be minimized into one modeled event"
    );
    assert_eq!(
        witness_json["normalized"]["event_batches"][0]["events"][0]["raw_events"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "three save events inside one debounce window should stay visible as one coalesced batch"
    );
    assert_eq!(
        witness_json["normalized"]["child_lifecycle_obligations"][0]["resolution"],
        "exited"
    );
    assert_eq!(
        witness_json["normalized"]["restart_decisions"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "events across two debounce windows should produce two restart decisions"
    );

    let replay = run_kobo(&[s("replay"), path_arg(&witness)], &project.root);
    assert_success(&replay, "imported watch trace witness should replay");
    assert_contains(
        &replay.stdout,
        r#""replay":"watch_trace_exact""#,
        "watch trace replay should confirm the witness hash",
    );
}

#[test]
fn watch_trace_replay_rejects_mutated_event_order_or_child_exit() {
    let project = TestProject::new("watchexec-trace-mutation");
    let trace = copy_watchexec_trace(&project);
    let witness = project.root.join(".kobo/witnesses/watchexec-policy.kwit");
    let import = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&trace),
            s("--witness-out"),
            path_arg(&witness),
        ],
        &project.root,
    );
    assert_success(&import, "watch trace import should succeed before mutation");

    let mut mutated_exit: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("trace witness should be readable"),
    )
    .expect("trace witness should parse");
    mutated_exit["raw_events"][7]["exit_code"] = Value::from(9);
    let mutated_exit_path = project.root.join(".kobo/witnesses/mutated-exit.kwit");
    fs::write(
        &mutated_exit_path,
        serde_json::to_vec_pretty(&mutated_exit).expect("mutated witness should serialize"),
    )
    .expect("mutated witness should write");

    let exit_replay = run_kobo(
        &[
            s("replay"),
            path_arg(&mutated_exit_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(&exit_replay, "mutated child exit must fail replay");
    assert_contains(
        &exit_replay.stdout,
        "K0104",
        "mutated child exit should report replay divergence",
    );

    let mut reordered: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("trace witness should be readable"),
    )
    .expect("trace witness should parse");
    let raw_events = reordered["raw_events"]
        .as_array_mut()
        .expect("raw events should be mutable");
    raw_events.swap(0, 1);
    let reordered_path = project.root.join(".kobo/witnesses/reordered.kwit");
    fs::write(
        &reordered_path,
        serde_json::to_vec_pretty(&reordered).expect("reordered witness should serialize"),
    )
    .expect("reordered witness should write");

    let order_replay = run_kobo(
        &[
            s("replay"),
            path_arg(&reordered_path),
            s("--error-format=json"),
        ],
        &project.root,
    );
    assert_failure(&order_replay, "mutated event order must fail replay");
    assert_contains(
        &order_replay.stdout,
        "watch trace witness diverged",
        "mutated event order should report trace divergence",
    );
}

#[test]
fn watch_trace_import_rejects_orphan_child_lifecycle() {
    let project = TestProject::new("watchexec-trace-orphan-child");
    let trace = project.write(
        "traces/orphan-child.json",
        &trace_with_events(
            r#"[
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 1,
      "timestamp_ms": 10,
      "duplicate_marker": "unique",
      "evidence_grade": "metadata_only"
    },
    {
      "kind": "timer_fired",
      "window": 1,
      "timestamp_ms": 210
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "timestamp_ms": 211
    },
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 212
    }
  ]"#,
        ),
    );
    let witness = project.root.join(".kobo/witnesses/orphan-child.kwit");

    let output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&trace),
            s("--witness-out"),
            path_arg(&witness),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "watch trace import should reject a started child without lifecycle resolution",
    );
    assert_contains(
        &output.combined(),
        "orphan child",
        "orphan child diagnostics should be actionable",
    );
}

#[test]
fn watch_trace_import_rejects_double_running_exclusive_child() {
    let project = TestProject::new("watchexec-trace-double-running");
    let trace = project.write(
        "traces/double-running.json",
        &trace_with_events(
            r#"[
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 1,
      "timestamp_ms": 10,
      "duplicate_marker": "unique",
      "evidence_grade": "metadata_only"
    },
    {
      "kind": "timer_fired",
      "window": 1,
      "timestamp_ms": 210
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "timestamp_ms": 211
    },
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 212
    },
    {
      "kind": "child_start",
      "child_id": "cmd-2",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 213
    }
  ]"#,
        ),
    );
    let witness = project.root.join(".kobo/witnesses/double-running.kwit");

    let output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&trace),
            s("--witness-out"),
            path_arg(&witness),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "watch trace import should reject exclusive double-running children",
    );
    assert_contains(
        &output.combined(),
        "double-running child",
        "double-running diagnostics should be actionable",
    );
}

#[test]
fn watch_trace_import_records_kill_resolution_as_lifecycle_boundary() {
    let project = TestProject::new("watchexec-trace-kill-resolution");
    let trace = project.write(
        "traces/kill-resolution.json",
        &trace_with_events(
            r#"[
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 1,
      "timestamp_ms": 10,
      "duplicate_marker": "unique",
      "evidence_grade": "metadata_only"
    },
    {
      "kind": "timer_fired",
      "window": 1,
      "timestamp_ms": 210
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "timestamp_ms": 211
    },
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 212
    },
    {
      "kind": "child_kill",
      "child_id": "cmd-1",
      "timestamp_ms": 300
    }
  ]"#,
        ),
    );
    let witness = project.root.join(".kobo/witnesses/kill-resolution.kwit");

    let output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&trace),
            s("--witness-out"),
            path_arg(&witness),
        ],
        &project.root,
    );
    assert_success(
        &output,
        "watch trace import should accept kill as an explicit lifecycle resolution",
    );
    let witness_json: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("kill-resolution witness should read"),
    )
    .expect("kill-resolution witness should parse");
    assert_eq!(
        witness_json["normalized"]["child_lifecycle_obligations"][0]["resolution"],
        "killed"
    );
}

#[test]
fn watch_trace_import_requires_fresh_watcher_process_and_time_summaries() {
    let project = TestProject::new("watchexec-trace-summary");
    let missing_time = project.write(
        "traces/missing_time.json",
        r#"{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": [
    {
      "kind": "watcher",
      "name": "notify-like",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["raw_event"],
      "modeled_facts": ["event_kind"],
      "unsupported_guarantees": ["global_total_order"],
      "replay_confidence": "metadata-only",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    },
    {
      "kind": "process",
      "name": "process-supervisor",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["spawn"],
      "modeled_facts": ["child_id"],
      "unsupported_guarantees": ["platform_signal_equivalence"],
      "replay_confidence": "modelled",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    }
  ],
  "events": []
}"#,
    );
    let output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&missing_time)],
        &project.root,
    );
    assert_failure(&output, "missing time adapter summary should fail import");
    assert_contains(
        &output.combined(),
        "missing adapter summary: time",
        "missing summaries should produce actionable diagnostics",
    );

    let stale_watcher = project.write(
        "traces/stale_watcher.json",
        &trace_with_events(valid_watchexec_events()).replace(
            r#""kind": "watcher",
      "name": "notify-like",
      "schema_version": 1"#,
            r#""kind": "watcher",
      "name": "notify-like",
      "schema_version": 0"#,
        ),
    );
    let stale = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&stale_watcher)],
        &project.root,
    );
    assert_failure(&stale, "stale watcher adapter summary should fail import");
    assert_contains(
        &stale.combined(),
        "stale adapter summary: watcher",
        "stale summaries should name the stale contract kind",
    );
}

#[test]
fn watch_trace_import_requires_external_comparison_evidence() {
    let project = TestProject::new("watchexec-trace-external-comparison");
    let missing_comparisons = project.write(
        "traces/missing_comparisons.json",
        &trace_with_events(valid_watchexec_events()).replace(
            r#",
  "external_comparisons": [
    {
      "implementation": "watchexec",
      "behavior": "coalesced filesystem events",
      "disposition": "replay_fixture",
      "reason": "normalized debounce batches preserve duplicate raw events",
      "evidence_anchor": "events[]"
    }
  ]"#,
            "",
        ),
    );
    let output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&missing_comparisons),
        ],
        &project.root,
    );
    assert_failure(
        &output,
        "missing external implementation comparison evidence should fail import",
    );
    assert_contains(
        &output.combined(),
        "missing external_comparisons",
        "trace import should require explicit external comparison evidence",
    );
}

#[test]
fn watch_trace_import_requires_adapter_cargo_feature_evidence() {
    let project = TestProject::new("watchexec-trace-feature-evidence");
    let missing_features = project.write(
        "traces/missing_features.json",
        r#"{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": [
    {
      "kind": "watcher",
      "name": "notify-like",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["raw_event"],
      "modeled_facts": ["event_kind"],
      "unsupported_guarantees": ["global_total_order"],
      "replay_confidence": "metadata-only",
      "source_map_anchor": "events[]"
    },
    {
      "kind": "process",
      "name": "process-supervisor",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["spawn"],
      "modeled_facts": ["child_id"],
      "unsupported_guarantees": ["platform_signal_equivalence"],
      "replay_confidence": "modelled",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    },
    {
      "kind": "time",
      "name": "logical-debounce",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["timer_fire"],
      "modeled_facts": ["window"],
      "unsupported_guarantees": ["real_os_time_determinism"],
      "replay_confidence": "partial",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"]
    }
  ],
  "events": []
}"#,
    );
    let output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&missing_features)],
        &project.root,
    );
    assert_failure(
        &output,
        "missing adapter Cargo feature evidence should fail import",
    );
    assert_contains(
        &output.combined(),
        "missing adapter cargo_features: watcher",
        "adapter summaries should require feature compatibility evidence",
    );
}

#[test]
fn watch_trace_import_catches_orphan_and_double_running_children() {
    let project = TestProject::new("watchexec-trace-supervisor");
    let orphan = project.write(
        "traces/orphan.json",
        &trace_with_events(
            r#"[
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 1
    }
  ]"#,
        ),
    );
    let orphan_output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&orphan)],
        &project.root,
    );
    assert_failure(&orphan_output, "orphan child trace should fail import");
    assert_contains(
        &orphan_output.combined(),
        "orphan child",
        "orphan child diagnostics should name the lifecycle issue",
    );

    let double_run = project.write(
        "traces/double_run.json",
        &trace_with_events(
            r#"[
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 1
    },
    {
      "kind": "child_start",
      "child_id": "cmd-2",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 2
    },
    {
      "kind": "child_exit",
      "child_id": "cmd-1",
      "exit_code": 0,
      "timestamp_ms": 3
    },
    {
      "kind": "child_exit",
      "child_id": "cmd-2",
      "exit_code": 0,
      "timestamp_ms": 4
    }
  ]"#,
        ),
    );
    let double_run_output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&double_run)],
        &project.root,
    );
    assert_failure(
        &double_run_output,
        "double-running exclusive child trace should fail import",
    );
    assert_contains(
        &double_run_output.combined(),
        "double-running child",
        "double-running diagnostics should name the supervisor race",
    );
}
