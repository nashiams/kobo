mod cli_test_support;

use std::fs;
use std::path::Path;
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
    mutated_exit["raw_events"][6]["exit_code"] = Value::from(9);
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
      "source_map_anchor": "events[]"
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
