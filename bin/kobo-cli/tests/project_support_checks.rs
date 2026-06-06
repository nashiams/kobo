mod cli_test_support;

use std::fs;
use std::path::Path;
use std::time::Duration;

use cli_test_support::{
    assert_contains, assert_failure, assert_success, path_arg, run_kobo_with_timeout, s,
    TestProject,
};
use kobo_proof::stable_hash;
use serde_json::Value;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

fn run_kobo(args: &[String], cwd: &Path) -> cli_test_support::CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn write_project_files(project: &TestProject) {
    project.write(
        "Cargo.toml",
        r#"[package]
name = "support-case"
version = "0.1.0"
edition = "2021"
rust-version = "1.78"

[features]
default = ["notify"]
polling = []

[dependencies]
notify = "6"
tokio = { version = "1", features = ["rt", "time"] }
ignore = "0.4"
clap = "4"
serde = { version = "1", features = ["derive"] }
tracing = "0.1"

[dev-dependencies]
insta = "1"
"#,
    );
    project.write("Kobo.toml", "[package]\nname = \"support-case\"\n");
    project.main_file("mod supervisor;\nfn main() {}\n");
    project.write("src/supervisor.kobo", "fn apply_policy() {}\n");
    project.write("src/adapters/notify.rs", "pub fn notify_adapter() {}\n");
    project.write("src/adapters/process.rs", "pub fn process_adapter() {}\n");
    project.write("examples/restart.kobo", "fn main() {}\n");
    project.write("tests/restart_behavior.kobo", "fn main() {}\n");
    write_upstream_inventory_files(project);
    write_evidence(
        project,
        ".kobo/evidence/upstream-inventory.json",
        "upstream_inventory",
        "inventory",
    );
    write_evidence(
        project,
        ".kobo/evidence/language.json",
        "language_surface",
        "language",
    );
    for (path, subject) in [
        (".kobo/evidence/filesystem-events.json", "filesystem_events"),
        (".kobo/evidence/watcher-backend.json", "watcher_backend"),
        (".kobo/evidence/paths.json", "paths"),
        (".kobo/evidence/process.json", "process_execution"),
        (".kobo/evidence/signals.json", "signals"),
        (".kobo/evidence/process-groups.json", "process_groups"),
        (".kobo/evidence/env.json", "environment_variables"),
        (".kobo/evidence/terminal.json", "terminal_io"),
        (".kobo/evidence/stdio.json", "stdio"),
        (".kobo/evidence/timers.json", "timers"),
    ] {
        write_evidence(project, path, "platform_model", subject);
    }
    for (path, subject) in [
        (".kobo/evidence/adapter-watcher.json", "watcher_backend"),
        (".kobo/evidence/adapter-async.json", "async_runtime"),
        (".kobo/evidence/adapter-process.json", "process_handling"),
        (".kobo/evidence/adapter-signals.json", "signal_handling"),
        (".kobo/evidence/adapter-ignore-path.json", "ignore_path"),
        (".kobo/evidence/adapter-config.json", "config"),
        (".kobo/evidence/adapter-cli.json", "cli"),
        (".kobo/evidence/adapter-serialization.json", "serialization"),
        (".kobo/evidence/adapter-logging.json", "logging_tracing"),
        (".kobo/evidence/adapter-errors.json", "errors"),
    ] {
        write_evidence(project, path, "adapter_summary", subject);
    }
    write_evidence(
        project,
        ".kobo/evidence/async.json",
        "async_runtime",
        "async",
    );
    write_evidence(
        project,
        ".kobo/evidence/generated-backend.json",
        "generated_backend",
        "backend",
    );
    for subject in [
        "upstream_tests",
        "kobo_replay_tests",
        "kobo_liveness_tests",
        "cli_behavior",
        "config_behavior",
        "exit_behavior",
        "logging_behavior",
        "package_behavior",
        "platform_behavior",
    ] {
        write_evidence(
            project,
            &format!(".kobo/evidence/parity-{subject}.json"),
            "test_release_parity",
            subject,
        );
    }
    for subject in ["startup", "steady_state", "restart", "memory", "binary"] {
        write_evidence(
            project,
            &format!(".kobo/evidence/perf-{subject}.json"),
            "performance",
            subject,
        );
    }
    write_evidence(
        project,
        ".kobo/evidence/upstream-tests.json",
        "upstream_tests",
        "upstream",
    );
    write_evidence(
        project,
        ".kobo/evidence/reviewer-a.json",
        "reviewer_report",
        "reviewer-a",
    );
    write_evidence(
        project,
        ".kobo/evidence/reviewer-b.json",
        "reviewer_report",
        "reviewer-b",
    );
    project.write(".kobo/evidence/release.zip", "release artifact bytes\n");
}

fn write_upstream_inventory_files(project: &TestProject) {
    project.write(
        "upstream/watchexec/Cargo.toml",
        r#"[workspace]
members = ["crates/cli", "crates/supervisor", "crates/platform"]

[workspace.package]
version = "1.0.0"
"#,
    );
    project.write(
        "upstream/watchexec/crates/cli/Cargo.toml",
        r#"[package]
name = "watchexec-cli"
version = "1.0.0"
edition = "2021"

[[bin]]
name = "watchexec"
path = "src/main.rs"

[features]
default = []
"#,
    );
    project.write(
        "upstream/watchexec/crates/supervisor/Cargo.toml",
        r#"[package]
name = "watchexec-supervisor"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[features]
default = []
polling = []
"#,
    );
    project.write(
        "upstream/watchexec/crates/platform/Cargo.toml",
        r#"[package]
name = "watchexec-platform"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/windows.rs"

[features]
default = []
"#,
    );
    project.write("upstream/watchexec/build.rs", "fn main() {}\n");
    project.write(
        "upstream/watchexec/crates/cli/src/main.rs",
        "pub fn cli() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/lib.rs",
        "pub struct Supervisor;\npub fn restart_policy() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/windows.rs",
        "pub fn windows_watch() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/unix.rs",
        "pub fn unix_watch() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/tests/cli.rs",
        "#[test]\nfn cli_flags() {}\n",
    );
    project.write("upstream/watchexec/examples/restart.rs", "fn main() {}\n");
    project.write("upstream/watchexec/fixtures/save.json", "{}\n");
    project.write("upstream/watchexec/target/release/watchexec", "binary\n");
}

fn write_evidence(project: &TestProject, path: &str, evidence_kind: &str, subject: &str) {
    let command = format!("kobo check {subject}");
    let transcript_path = path.replace(".json", ".transcript.json");
    let transcript = format!(
        r#"{{
  "schema_version": 1,
  "command": "{command}",
  "status": "passed",
  "exit_code": 0,
  "stdout": "{subject} ok",
  "stderr": ""
}}"#
    );
    project.write(&transcript_path, &transcript);
    let transcript_hash = stable_hash(&transcript);
    project.write(
        path,
        &format!(
            r#"{{
  "schema_version": 1,
  "evidence_kind": "{evidence_kind}",
  "subject": "{subject}",
  "checks": ["parse", "check", "lower", "source_map", "rare_diagnostics"],
  "covered_paths": ["crates/cli/src/main.rs", "crates/supervisor/src/lib.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],
  "tested_platforms": ["windows", "macos", "linux"],
  "commands": [
    {{"command": "{command}", "status": "passed", "transcript_path": "{transcript_path}", "output_hash": "{transcript_hash}"}}
  ],
  "behavior_tests": ["watcher", "restart", "signal", "stdin", "path_filter"],
  "conformance_results": [
    {{"name": "{subject}-conformance", "status": "passed"}}
  ],
  "stale_check": {{"status": "passed", "crate_version": "1.0.0", "features": ["default", "polling", "rt", "time", "derive"]}},
  "mutation_results": ["task-order", "timer-order", "cancel-order", "channel-delivery"],
  "scheduler_facts": ["watcher-batching", "restart-ordering", "signal-delivery", "child-exit-race"],
  "decision": "equivalent_or_stronger"
}}"#
        ),
    );
}

fn write_complete_support_manifest(project: &TestProject) {
    project.write(
        ".kobo/project-support.json",
        r#"{
  "schema_version": 1,
  "claim": "project_support",
  "upstream_inventory": {
    "root": "upstream/watchexec",
    "workspace_manifest": "Cargo.toml",
    "crate_tree": ["crates/cli/src/main.rs", "crates/supervisor/src/lib.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],
    "modules": ["crates/cli/src/main.rs", "crates/supervisor/src/lib.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],
    "public_types": ["Supervisor", "RestartPolicy", "WatchEvent"],
    "cli_surfaces": ["watchexec --restart", "watchexec --watch", "watchexec --signal"],
    "test_fixtures": ["crates/cli/tests/cli.rs", "fixtures/save.json"],
    "platform_paths": [
      {"path": "crates/platform/src/windows.rs", "disposition": "formal_adapter", "correctness_relevant": true, "source_preserved": true},
      {"path": "crates/platform/src/unix.rs", "disposition": "formal_adapter", "correctness_relevant": true, "source_preserved": true}
    ],
    "feature_combinations": [["default"], ["default", "polling"]],
    "examples": ["examples/restart.rs"],
    "build_scripts": ["build.rs"],
    "release_artifacts": ["target/release/watchexec"],
    "evidence_path": ".kobo/evidence/upstream-inventory.json"
  },
  "language_surface": {
    "constructs": [
      "modules",
      "workspace",
      "feature_flags",
      "platform_cfg",
      "generics",
      "traits",
      "async_code",
      "error_types",
      "iterators",
      "tests",
      "examples",
      "public_api",
      "build_scripts",
      "release_artifacts"
    ],
    "parse": true,
    "check": true,
    "lower": true,
    "source_map": true,
    "rare_diagnostics": true,
    "feature_matrix": [["default"], ["default", "polling"]],
    "evidence_path": ".kobo/evidence/language.json"
  },
  "platform_models": [
    {"kind": "filesystem_events", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["create", "modify", "delete", "move", "rename", "close", "metadata", "rescan", "synthetic", "duplicate", "coalesced"], "evidence_path": ".kobo/evidence/filesystem-events.json"},
    {"kind": "watcher_backend", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["windows-api", "fsevents", "kqueue", "inotify", "polling"], "evidence_path": ".kobo/evidence/watcher-backend.json"},
    {"kind": "paths", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["case-sensitivity", "case-only-rename", "canonical-recovery", "symlink-policy", "project-root", "network-fallback"], "evidence_path": ".kobo/evidence/paths.json"},
    {"kind": "process_execution", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["spawn", "args", "env", "cwd", "shell", "no-shell", "detached"], "evidence_path": ".kobo/evidence/process.json"},
    {"kind": "signals", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["stop", "interrupt", "kill-fallback", "graceful-shutdown", "kill-timeout", "unsupported-signal"], "evidence_path": ".kobo/evidence/signals.json"},
    {"kind": "process_groups", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["process-groups", "sessions", "job-objects", "child-trees"], "evidence_path": ".kobo/evidence/process-groups.json"},
    {"kind": "environment_variables", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["inheritance", "changed-path-delivery"], "evidence_path": ".kobo/evidence/env.json"},
    {"kind": "terminal_io", "platforms": ["windows", "macos", "linux"], "replay_grade": "sampled", "behaviors": ["tty", "handles", "log-forwarding"], "evidence_path": ".kobo/evidence/terminal.json"},
    {"kind": "stdio", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["stdin", "stdout", "stderr", "routing"], "evidence_path": ".kobo/evidence/stdio.json"},
    {"kind": "timers", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled", "behaviors": ["debounce", "delay-run", "stop-timeout", "poll-interval", "cancellation", "timeout-firing"], "evidence_path": ".kobo/evidence/timers.json"}
  ],
  "adapters": [
    {"kind": "watcher_backend", "name": "notify", "crate_source": {"kind": "cargo_dependency", "name": "notify"}, "version_range": "^6", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["event-kind", "backend"], "unsupported_guarantees": ["global-total-order"], "conformance_tests": ["duplicate-events"], "replay_evidence": ".kobo/evidence/adapter-watcher.json", "stale_summary_detection": true},
    {"kind": "async_runtime", "name": "tokio", "crate_source": {"kind": "cargo_dependency", "name": "tokio"}, "version_range": "^1", "cargo_features": ["rt", "time"], "summary_version": 1, "modeled_facts": ["task-order"], "unsupported_guarantees": ["arbitrary-scheduler-equivalence"], "conformance_tests": ["cancel-order"], "replay_evidence": ".kobo/evidence/adapter-async.json", "stale_summary_detection": true},
    {"kind": "process_handling", "name": "std::process", "crate_source": {"kind": "std", "name": "std::process"}, "version_range": "std", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["spawn", "wait"], "unsupported_guarantees": ["platform-signal-equivalence"], "conformance_tests": ["exclusive-child"], "replay_evidence": ".kobo/evidence/adapter-process.json", "stale_summary_detection": true},
    {"kind": "signal_handling", "name": "process-adapter", "crate_source": {"kind": "project_module", "path": "src/adapters/process.rs"}, "version_range": "project", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["interrupt", "kill"], "unsupported_guarantees": ["windows-posix-equivalence"], "conformance_tests": ["interrupt-before-kill"], "replay_evidence": ".kobo/evidence/adapter-signals.json", "stale_summary_detection": true},
    {"kind": "ignore_path", "name": "ignore", "crate_source": {"kind": "cargo_dependency", "name": "ignore"}, "version_range": "^0.4", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["gitignore", "case"], "unsupported_guarantees": ["remote-fs-canonicalization"], "conformance_tests": ["absolute-ignore"], "replay_evidence": ".kobo/evidence/adapter-ignore-path.json", "stale_summary_detection": true},
    {"kind": "config", "name": "kobo-config", "crate_source": {"kind": "project_module", "path": "Kobo.toml"}, "version_range": "project", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["reload"], "unsupported_guarantees": ["external-editor-atomicity"], "conformance_tests": ["reload"], "replay_evidence": ".kobo/evidence/adapter-config.json", "stale_summary_detection": true},
    {"kind": "cli", "name": "clap", "crate_source": {"kind": "cargo_dependency", "name": "clap"}, "version_range": "^4", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["parse"], "unsupported_guarantees": ["shell-quoting-equivalence"], "conformance_tests": ["flags"], "replay_evidence": ".kobo/evidence/adapter-cli.json", "stale_summary_detection": true},
    {"kind": "serialization", "name": "serde", "crate_source": {"kind": "cargo_dependency", "name": "serde"}, "version_range": "^1", "cargo_features": ["derive"], "summary_version": 1, "modeled_facts": ["json"], "unsupported_guarantees": ["format-autodetect"], "conformance_tests": ["witness-json"], "replay_evidence": ".kobo/evidence/adapter-serialization.json", "stale_summary_detection": true},
    {"kind": "logging_tracing", "name": "tracing", "crate_source": {"kind": "cargo_dependency", "name": "tracing"}, "version_range": "^0.1", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["events"], "unsupported_guarantees": ["terminal-color-equivalence"], "conformance_tests": ["log-routing"], "replay_evidence": ".kobo/evidence/adapter-logging.json", "stale_summary_detection": true},
    {"kind": "errors", "name": "kobo-errors", "crate_source": {"kind": "project_module", "path": "src/main.kobo"}, "version_range": "project", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["source-mapped"], "unsupported_guarantees": ["foreign-panic-shape"], "conformance_tests": ["diagnostics"], "replay_evidence": ".kobo/evidence/adapter-errors.json", "stale_summary_detection": true}
  ],
  "async_runtime": {
    "model": "versioned_tokio_adapter",
    "semantics": ["spawn", "join", "cancel", "select", "timer", "channel", "backpressure", "shutdown", "blocking"],
    "scheduler_facts": ["watcher-batching", "restart-ordering", "signal-delivery", "child-exit-race"],
    "mutation_checks": ["task-order", "timer-order", "cancel-order", "channel-delivery"],
    "conformance_evidence": ".kobo/evidence/async.json"
  },
  "generated_backend": {
    "reviewable": true,
    "deterministic": true,
    "source_mapped": true,
    "diagnostics_on_kobo_source": true,
    "replay_on_kobo_source": true,
    "debt_on_kobo_source": true,
    "proof_on_kobo_source": true,
    "lsp_on_kobo_source": true,
    "targets": ["windows", "macos", "linux"],
    "evidence_path": ".kobo/evidence/generated-backend.json"
  },
  "test_release_parity": {
    "upstream_tests": ".kobo/evidence/parity-upstream_tests.json",
    "kobo_replay_tests": ".kobo/evidence/parity-kobo_replay_tests.json",
    "kobo_liveness_tests": ".kobo/evidence/parity-kobo_liveness_tests.json",
    "cli_behavior": ".kobo/evidence/parity-cli_behavior.json",
    "config_behavior": ".kobo/evidence/parity-config_behavior.json",
    "exit_behavior": ".kobo/evidence/parity-exit_behavior.json",
    "logging_behavior": ".kobo/evidence/parity-logging_behavior.json",
    "package_behavior": ".kobo/evidence/parity-package_behavior.json",
    "platform_behavior": ".kobo/evidence/parity-platform_behavior.json",
    "performance": {
      "startup": ".kobo/evidence/perf-startup.json",
      "steady_state": ".kobo/evidence/perf-steady_state.json",
      "restart": ".kobo/evidence/perf-restart.json",
      "memory": ".kobo/evidence/perf-memory.json",
      "binary": ".kobo/evidence/perf-binary.json"
    },
    "release_artifacts": [".kobo/evidence/release.zip"]
  },
  "proof_debt_map": [
    {"module": "src/supervisor.kobo", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "src/adapters/notify.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "src/adapters/process.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "examples/restart.kobo", "classification": "debt", "criticality": "non-critical", "release_blocking": false, "justification": "example-only trace parity is not release blocking"}
  ],
  "independent_equivalence": {
    "original_upstream_tests": ".kobo/evidence/upstream-tests.json",
    "mutation_tests": ["watcher", "child", "cancellation", "config", "platform"],
    "reviewer_reports": [
      {"reviewer": "reviewer-a", "evidence_path": ".kobo/evidence/reviewer-a.json"},
      {"reviewer": "reviewer-b", "evidence_path": ".kobo/evidence/reviewer-b.json"}
    ]
  }
}"#,
    );
}

fn parse_stdout_json(output: &cli_test_support::CliOutput) -> Value {
    serde_json::from_str(&output.stdout).expect("stdout should be JSON")
}

fn write_ready_supervisor_slice_state(project: &TestProject) {
    project.write(
        ".kobo/watch/source-watch.json",
        r#"{
  "schema_version": 1,
  "mode": "source_watch_state",
  "scenarios": [
    "save_burst",
    "multi_window",
    "child_exit_race",
    "shutdown_pending_timer",
    "signal_process_group",
    "changed_path_delivery"
  ],
  "watcher_evidence": "modeled",
  "event_batches": [
    {
      "id": "watch-batch-1",
      "replay_grade": "modeled",
      "events": [
        {
          "kind": "modify",
          "paths": [{"role": "source_path", "path": "src/main.kobo"}],
          "duplicate_or_coalesced": "unique",
          "evidence_grade": "modeled"
        },
        {
          "kind": "modify",
          "paths": [{"role": "source_path", "path": "src/main.kobo"}],
          "duplicate_or_coalesced": "duplicate",
          "evidence_grade": "modeled"
        },
        {
          "kind": "rename",
          "paths": [
            {"role": "source_path", "path": "src/.main.kobo.tmp"},
            {"role": "destination_path", "path": "src/main.kobo"}
          ],
          "duplicate_or_coalesced": "coalesced",
          "evidence_grade": "modeled"
        }
      ]
    },
    {
      "id": "watch-batch-2",
      "replay_grade": "modeled",
      "events": [
        {
          "kind": "modify",
          "paths": [{"role": "source_path", "path": "src/supervisor.kobo"}],
          "duplicate_or_coalesced": "unique",
          "evidence_grade": "modeled"
        }
      ]
    }
  ],
  "debounce_windows": [
    {
      "id": "watch-window-1",
      "timer_evidence": "modeled",
      "replay_grade": "modeled",
      "event_batch_ids": ["watch-batch-1"],
      "fired": true
    },
    {
      "id": "watch-window-2",
      "timer_evidence": "modeled",
      "replay_grade": "modeled",
      "event_batch_ids": ["watch-batch-2"],
      "fired": true
    }
  ],
  "restart_decisions": [
    {
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "rerun",
      "selected_by": ["src/main.kobo"],
      "changed_path_delivery": ["env", "stdin"]
    },
    {
      "policy_branch": "watchexec.restart.child_exit_race",
      "action": "rerun_after_exit",
      "selected_by": ["src/supervisor.kobo"],
      "changed_path_delivery": ["env", "stdin"]
    }
  ],
  "child_lifecycle_obligations": [
    {
      "command_kind": "in_process_check",
      "resolution": "in_process_rerun_finished",
      "evidence_grade": "modeled"
    },
    {
      "command_kind": "external_child",
      "resolution": "child_exit_observed",
      "evidence_grade": "modeled"
    },
    {
      "command_kind": "external_child",
      "resolution": "process_group_signaled",
      "evidence_grade": "modeled"
    },
    {
      "command_kind": "external_child",
      "resolution": "shutdown_child_waited",
      "evidence_grade": "modeled"
    }
  ],
  "shutdown_resolutions": [
    {"kind": "pending_timer", "resolution": "cancelled_before_exit", "evidence_grade": "modeled"},
    {"kind": "active_child", "resolution": "waited_before_exit", "evidence_grade": "modeled"}
  ],
  "signal_process_group": {
    "signal": "interrupt",
    "process_group": true,
    "fallback": "kill_timeout",
    "evidence_grade": "modeled"
  }
}"#,
    );
}

#[test]
fn doctor_project_support_reports_complete_general_project_support() {
    let project = TestProject::new("doctor-project-support-ready");
    write_project_files(&project);
    write_complete_support_manifest(&project);

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "project support report should succeed");
    let value = parse_stdout_json(&output);
    assert_eq!(value["command"], "doctor --project-support");
    assert_eq!(
        value["project_support"]["status"],
        "ready",
        "ready project blockers: {}",
        value["project_support"]["blockers"]
    );
    assert_eq!(value["project_support"]["claim"], "project_support");
    assert_contains(
        &value["project_support"]["inventory"].to_string(),
        "supervisor.kobo",
        "inventory should include Kobo-owned source modules",
    );
    assert_contains(
        &value["project_support"]["inventory"].to_string(),
        "notify.rs",
        "inventory should include Rust adapter modules",
    );
    for expected in [
        "filesystem_events",
        "process_execution",
        "signals",
        "async_runtime",
        "generated_backend",
        "test_release_parity",
        "independent_equivalence",
    ] {
        assert_contains(
            &value.to_string(),
            expected,
            "support report should expose the general project-support surface",
        );
    }

    let gate = run_kobo(
        &[
            s("doctor"),
            s("--project-support"),
            s("--require-ready"),
            s("--json"),
        ],
        &project.root,
    );
    assert_success(&gate, "ready project should pass the CI support gate");
}

#[test]
fn doctor_supervisor_slice_ci_gate_only_requires_kobo_owned_slice_evidence() {
    let project = TestProject::new("doctor-supervisor-slice-ready");
    project.main_file("fn main() {}\n");
    write_ready_supervisor_slice_state(&project);

    let output = run_kobo(
        &[s("doctor"), s("--supervisor-slice"), s("--json")],
        &project.root,
    );
    assert_success(&output, "supervisor slice report should succeed");
    let value = parse_stdout_json(&output);
    assert_eq!(value["command"], "doctor --supervisor-slice");
    assert_eq!(value["supervisor_slice"]["status"], "ready");
    assert_eq!(
        value["supervisor_slice"]["surfaces"]["watcher_events"],
        Value::from(4)
    );
    assert_contains(
        &value.to_string(),
        "kobo-owned watcher/supervisor slice",
        "slice gate should name the narrow CI scope",
    );

    let gate = run_kobo(
        &[
            s("doctor"),
            s("--supervisor-slice"),
            s("--require-ready"),
            s("--json"),
        ],
        &project.root,
    );
    assert_success(&gate, "ready supervisor slice should pass the CI gate");

    let checked_in_state = TestProject::repo_root()
        .join("tests")
        .join("fixtures")
        .join("watchexec_supervisor_policy")
        .join("source_watch_ready.json");
    let fixture_gate = run_kobo(
        &[
            s("doctor"),
            s("--supervisor-slice"),
            s("--supervisor-slice-state"),
            path_arg(&checked_in_state),
            s("--require-ready"),
            s("--json"),
        ],
        &project.root,
    );
    assert_success(
        &fixture_gate,
        "checked-in supervisor slice fixture should pass the CI gate",
    );
}

#[test]
fn doctor_supervisor_slice_ci_gate_blocks_unresolved_slice_debt() {
    let project = TestProject::new("doctor-supervisor-slice-blocked");
    project.main_file("fn main() {}\n");
    write_ready_supervisor_slice_state(&project);
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state = fs::read_to_string(&state_path).expect("slice state should read");
    fs::write(
        &state_path,
        state
            .replace(r#""replay_grade": "modeled""#, r#""replay_grade": "debt""#)
            .replace(
                r#""resolution": "in_process_rerun_finished""#,
                r#""resolution": "unresolved_started_child""#,
            ),
    )
    .expect("slice state should write");

    let report = run_kobo(
        &[s("doctor"), s("--supervisor-slice"), s("--json")],
        &project.root,
    );
    assert_success(&report, "blocked slice report should stay inspectable");
    let value = parse_stdout_json(&report);
    assert_eq!(value["supervisor_slice"]["status"], "blocked");
    assert_contains(
        &value["supervisor_slice"]["blockers"].to_string(),
        "watcher batch replay grade is debt",
        "slice gate should name debt-grade watcher evidence",
    );
    assert_contains(
        &value["supervisor_slice"]["blockers"].to_string(),
        "unresolved child lifecycle obligation",
        "slice gate should name unresolved child lifecycle evidence",
    );

    let gate = run_kobo(
        &[
            s("doctor"),
            s("--supervisor-slice"),
            s("--require-ready"),
            s("--json"),
        ],
        &project.root,
    );
    assert_failure(&gate, "blocked supervisor slice should fail the CI gate");
    assert_contains(
        &gate.stdout,
        "doctor supervisor slice gate blocked",
        "failed gate should return machine-readable blocker output",
    );
}

#[test]
fn doctor_supervisor_slice_ci_gate_blocks_metadata_only_readiness() {
    let project = TestProject::new("doctor-supervisor-slice-metadata-only");
    project.main_file("fn main() {}\n");
    write_ready_supervisor_slice_state(&project);
    let state_path = project.root.join(".kobo/watch/source-watch.json");
    let state = fs::read_to_string(&state_path).expect("slice state should read");
    fs::write(
        &state_path,
        state
            .replace(
                r#""watcher_evidence": "modeled""#,
                r#""watcher_evidence": "metadata-only""#,
            )
            .replace(
                r#""timer_evidence": "modeled""#,
                r#""timer_evidence": "metadata-only""#,
            ),
    )
    .expect("slice state should write");

    let report = run_kobo(
        &[s("doctor"), s("--supervisor-slice"), s("--json")],
        &project.root,
    );
    assert_success(
        &report,
        "metadata-only supervisor report should stay inspectable",
    );
    let value = parse_stdout_json(&report);
    assert_eq!(value["supervisor_slice"]["status"], "blocked");
    assert_contains(
        &value["supervisor_slice"]["blockers"].to_string(),
        "watcher evidence is metadata-only",
        "slice gate should reject metadata-only watcher evidence",
    );
    assert_contains(
        &value["supervisor_slice"]["blockers"].to_string(),
        "timer=metadata-only",
        "slice gate should reject metadata-only timer evidence",
    );
}

#[test]
fn doctor_project_support_gate_blocks_missing_or_opaque_critical_evidence() {
    let project = TestProject::new("doctor-project-support-blocked");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest
            .replace(
                r#""kind": "signals", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled""#,
                r#""kind": "signals", "platforms": ["windows", "macos", "linux"], "replay_grade": "opaque""#,
            )
            .replace(
                r#"{"module": "src/adapters/process.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false}"#,
                r#"{"module": "src/adapters/process.rs", "classification": "debt", "criticality": "correctness-critical", "release_blocking": false}"#,
            ),
    )
    .expect("support manifest should write");

    let report = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&report, "blocked report should still be inspectable");
    let value = parse_stdout_json(&report);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "critical platform model signals is opaque",
        "opaque critical platform evidence should be a named blocker",
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "critical module src/adapters/process.rs is classified as debt",
        "correctness-critical debt should be a named blocker",
    );

    let gate = run_kobo(
        &[
            s("doctor"),
            s("--project-support"),
            s("--require-ready"),
            s("--json"),
        ],
        &project.root,
    );
    assert_failure(&gate, "blocked project should fail the CI support gate");
    assert_contains(
        &gate.stdout,
        "doctor project support gate blocked",
        "failed gate should return machine-readable blocker output",
    );
}

#[test]
fn doctor_project_support_names_missing_required_surfaces() {
    let project = TestProject::new("doctor-project-support-missing");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest
            .replace(
                r#"      "modules",
"#,
                r#"      "__removed__",
"#,
            )
            .replace(
                r#"{"kind": "async_runtime", "name": "tokio", "crate_source": {"kind": "cargo_dependency", "name": "tokio"}, "version_range": "^1""#,
                r#"{"kind": "async_runtime", "name": "tokio", "crate_source": {"kind": "cargo_dependency", "name": "tokio"}, "version_range": """#,
            )
            .replace(
                r#""release_artifacts": [".kobo/evidence/release.zip"]"#,
                r#""release_artifacts": []"#,
            ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "missing-surface report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    for expected in [
        "missing language construct modules",
        "adapter async_runtime has empty version_range",
        "missing release artifact evidence",
    ] {
        assert_contains(
            &value["project_support"]["blockers"].to_string(),
            expected,
            "project support report should name missing required surfaces",
        );
    }
}

#[test]
fn doctor_project_support_rejects_empty_evidence_documents() {
    let project = TestProject::new("doctor-project-support-empty-evidence");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    project.write(".kobo/evidence/language.json", "{}\n");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "empty-evidence report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "evidence must be a non-empty JSON object",
        "project support gate should reject shape-only evidence",
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "evidence must include command results",
        "project support gate should require command-backed evidence",
    );
}

#[test]
fn doctor_project_support_rejects_missing_feature_combinations() {
    let project = TestProject::new("doctor-project-support-feature-matrix");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""feature_matrix": [["default"], ["default", "polling"]]"#,
            r#""feature_matrix": [["default"]]"#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "feature-matrix report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "language feature_matrix missing feature combination default+polling",
        "project support gate should require upstream feature combinations",
    );
}

#[test]
fn doctor_project_support_rejects_forged_command_transcript_hashes() {
    let project = TestProject::new("doctor-project-support-forged-transcript");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    project.write(
        ".kobo/evidence/language.transcript.json",
        r#"{
  "schema_version": 1,
  "command": "kobo check language",
  "status": "passed",
  "exit_code": 0,
  "stdout": "tampered",
  "stderr": ""
}"#,
    );

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "forged-transcript report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "evidence transcript hash mismatch",
        "project support gate should bind command evidence to transcript hashes",
    );
}

#[test]
fn doctor_project_support_derives_upstream_targets_from_cargo_metadata() {
    let project = TestProject::new("doctor-project-support-cargo-metadata");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""crate_tree": ["crates/cli/src/main.rs", "crates/supervisor/src/lib.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],
    "modules": ["crates/cli/src/main.rs", "crates/supervisor/src/lib.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],"#,
            r#""crate_tree": ["crates/cli/src/main.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],
    "modules": ["crates/cli/src/main.rs", "crates/platform/src/windows.rs", "crates/platform/src/unix.rs"],"#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "cargo-metadata report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "upstream inventory missing Cargo metadata target crates/supervisor/src/lib.rs",
        "project support gate should derive workspace target coverage from Cargo metadata",
    );
}

#[test]
fn doctor_project_support_rejects_duplicate_reviewer_evidence_paths() {
    let project = TestProject::new("doctor-project-support-duplicate-reviewer");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#"{"reviewer": "reviewer-b", "evidence_path": ".kobo/evidence/reviewer-b.json"}"#,
            r#"{"reviewer": "reviewer-b", "evidence_path": ".kobo/evidence/reviewer-a.json"}"#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "duplicate-reviewer report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "reviewer reports must use distinct evidence paths",
        "project support gate should reject duplicate reviewer evidence files",
    );
}

#[test]
fn doctor_project_support_rejects_stale_adapter_and_metadata_only_platform() {
    let project = TestProject::new("doctor-project-support-stale-adapter");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""kind": "filesystem_events", "platforms": ["windows", "macos", "linux"], "replay_grade": "modeled""#,
            r#""kind": "filesystem_events", "platforms": ["windows", "macos", "linux"], "replay_grade": "metadata-only""#,
        ),
    )
    .expect("support manifest should write");
    let adapter_path = project.root.join(".kobo/evidence/adapter-watcher.json");
    let adapter_evidence = fs::read_to_string(&adapter_path).expect("adapter evidence should read");
    fs::write(
        &adapter_path,
        adapter_evidence.replace(
            r#""stale_check": {"status": "passed""#,
            r#""stale_check": {"status": "failed""#,
        ),
    )
    .expect("adapter evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "stale-adapter report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "critical platform model filesystem_events is metadata-only",
        "project support gate should reject metadata-only critical platform evidence",
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "adapter watcher_backend stale check did not pass",
        "project support gate should reject stale adapter evidence",
    );
}

#[test]
fn doctor_project_support_rejects_adapter_source_not_declared_in_cargo() {
    let project = TestProject::new("doctor-project-support-adapter-source");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""kind": "async_runtime", "name": "tokio", "crate_source": {"kind": "cargo_dependency", "name": "tokio"}"#,
            r#""kind": "async_runtime", "name": "tokio", "crate_source": {"kind": "cargo_dependency", "name": "tokio-missing"}"#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "adapter-source report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "adapter async_runtime cargo dependency is not declared: tokio-missing",
        "project support gate should verify cargo-backed adapter sources",
    );
}
