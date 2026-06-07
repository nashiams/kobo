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

struct EvidenceCommandTranscript {
    command: String,
    argv_json: String,
    cwd: Option<String>,
    output_match: &'static str,
    source: String,
    hash: String,
}

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
    let inventory_command = build_evidence_command_transcript(
        project,
        &[
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
            "upstream/watchexec/Cargo.toml",
        ],
        None,
    );
    let language_command = build_evidence_command_transcript(
        project,
        &["cargo", "check", "--workspace", "--all-targets"],
        Some("upstream/watchexec"),
    );
    let behavior_command = build_evidence_command_transcript(
        project,
        &[
            "cargo",
            "test",
            "--workspace",
            "--all-targets",
            "--",
            "--nocapture",
        ],
        Some("upstream/watchexec"),
    );
    write_evidence(
        project,
        ".kobo/evidence/upstream-inventory.json",
        "upstream_inventory",
        "inventory",
        &inventory_command,
    );
    write_evidence(
        project,
        ".kobo/evidence/language.json",
        "language_surface",
        "language",
        &language_command,
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
        write_evidence(project, path, "platform_model", subject, &behavior_command);
    }
    for (path, subject) in [
        (".kobo/evidence/adapter-watcher.json", "watcher_backend"),
        (".kobo/evidence/adapter-async.json", "async_runtime"),
        (".kobo/evidence/adapter-process.json", "process_handling"),
        (".kobo/evidence/adapter-signals.json", "signal_handling"),
        (".kobo/evidence/adapter-ignore-path.json", "ignore_path"),
        (".kobo/evidence/adapter-config.json", "config"),
        (".kobo/evidence/adapter-cli.json", "cli"),
        (".kobo/evidence/adapter-shell.json", "shell_parsing"),
        (".kobo/evidence/adapter-serialization.json", "serialization"),
        (".kobo/evidence/adapter-logging.json", "logging_tracing"),
        (".kobo/evidence/adapter-terminal.json", "terminal_helpers"),
        (".kobo/evidence/adapter-errors.json", "errors"),
    ] {
        write_evidence(project, path, "adapter_summary", subject, &behavior_command);
    }
    write_evidence(
        project,
        ".kobo/evidence/async.json",
        "async_runtime",
        "async_runtime",
        &behavior_command,
    );
    write_evidence(
        project,
        ".kobo/evidence/generated-backend.json",
        "generated_backend",
        "backend",
        &language_command,
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
        "install_behavior",
    ] {
        write_evidence(
            project,
            &format!(".kobo/evidence/parity-{subject}.json"),
            "test_release_parity",
            subject,
            &behavior_command,
        );
    }
    for subject in [
        "startup",
        "steady_state",
        "restart",
        "memory",
        "binary",
        "watch_tree_scaling",
        "event_burst_scaling",
    ] {
        write_evidence(
            project,
            &format!(".kobo/evidence/perf-{subject}.json"),
            "performance",
            subject,
            &behavior_command,
        );
    }
    write_evidence(
        project,
        ".kobo/evidence/upstream-tests.json",
        "upstream_tests",
        "upstream",
        &behavior_command,
    );
    write_evidence(
        project,
        ".kobo/evidence/reviewer-a.json",
        "reviewer_report",
        "reviewer-a",
        &behavior_command,
    );
    write_evidence(
        project,
        ".kobo/evidence/reviewer-b.json",
        "reviewer_report",
        "reviewer-b",
        &behavior_command,
    );
    for subject in [
        "debt_summary",
        "proof_report",
        "replay_report",
        "inspect_output",
    ] {
        write_evidence(
            project,
            &format!(".kobo/evidence/proof-debt-{subject}.json"),
            "proof_debt_report",
            subject,
            &behavior_command,
        );
    }
    project.write(".kobo/evidence/release.zip", "release artifact bytes\n");
}

fn write_upstream_inventory_files(project: &TestProject) {
    project.write(
        "upstream/watchexec/Cargo.toml",
        r#"[workspace]
members = [
  "crates/lib",
  "crates/events",
  "crates/cli",
  "crates/supervisor",
  "crates/platform",
  "crates/signals",
  "crates/ignore-files",
  "crates/config",
  "crates/logging",
  "crates/errors",
]
resolver = "2"

[workspace.package]
version = "1.0.0"
"#,
    );
    project.write(
        "upstream/watchexec/crates/lib/Cargo.toml",
        r#"[package]
name = "watchexec"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[features]
default = []
"#,
    );
    project.write(
        "upstream/watchexec/crates/events/Cargo.toml",
        r#"[package]
name = "watchexec-events"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[features]
default = []
serde = []
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

[lib]
path = "src/lib.rs"

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
signals = []
"#,
    );
    project.write(
        "upstream/watchexec/crates/platform/Cargo.toml",
        r#"[package]
name = "watchexec-platform"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[features]
default = []
polling = []
"#,
    );
    for crate_name in ["signals", "ignore-files", "config", "logging", "errors"] {
        project.write(
            &format!("upstream/watchexec/crates/{crate_name}/Cargo.toml"),
            &format!(
                r#"[package]
name = "watchexec-{crate_name}"
version = "1.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[features]
default = []
"#
            ),
        );
    }
    project.write("upstream/watchexec/build.rs", "fn main() {}\n");
    project.write(
        "upstream/watchexec/crates/lib/src/lib.rs",
        "pub struct Watchexec;\npub fn configure() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/watchexec.rs",
        "pub fn run_watchexec() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/paths.rs",
        "pub fn normalize_path() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/late_join_set.rs",
        "pub fn join_tasks() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/action/worker.rs",
        "pub fn run_worker() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/action/return.rs",
        "pub fn classify_exit() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/sources/fs.rs",
        "pub fn read_fs_events() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/lib/src/sources/signal.rs",
        "pub fn read_signals() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/events/src/lib.rs",
        "pub struct Event;\npub fn normalize_event() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/events/src/fs.rs",
        "pub fn filesystem_event() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/events/src/serde_formats.rs",
        "pub fn serialize_event() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/src/main.rs",
        "fn main() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/src/lib.rs",
        "pub struct CliSurface;\npub fn parse_restart_flag() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/src/args/logging.rs",
        "pub fn parse_logging() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/src/config.rs",
        "pub fn read_config() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/lib.rs",
        "pub mod debounce;\npub mod policy;\npub struct Supervisor;\npub struct RestartPolicy;\npub fn restart_policy() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/command.rs",
        "pub fn spawn_command() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/errors.rs",
        "pub fn supervisor_error() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/job/state.rs",
        "pub fn job_state() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/policy.rs",
        "pub struct WatchEvent;\npub fn decide_restart() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/src/debounce.rs",
        "pub struct DebounceWindow;\npub fn coalesce() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/lib.rs",
        "pub mod linux;\npub mod macos;\npub mod polling;\npub mod windows;\npub struct PlatformModel;\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/windows.rs",
        "pub fn windows_watch() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/linux.rs",
        "pub fn linux_watch() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/macos.rs",
        "pub fn macos_watch() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/src/polling.rs",
        "pub fn polling_watch() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/signals/src/lib.rs",
        "pub struct SignalPlan;\npub fn interrupt_group() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/ignore-files/src/lib.rs",
        "pub struct IgnoreMatcher;\npub fn matches_path() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/config/src/lib.rs",
        "pub struct ConfigSource;\npub fn reload_config() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/logging/src/lib.rs",
        "pub struct LogEvent;\npub fn forward_log() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/errors/src/lib.rs",
        "pub struct ErrorReport;\npub fn report_error() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/tests/cli_flags.rs",
        "#[test]\nfn cli_flags() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/supervisor/tests/restart.rs",
        "#[test]\nfn restart_policy() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/platform/tests/platform.rs",
        "#[test]\nfn platform_model() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/signals/tests/signals.rs",
        "#[test]\nfn signal_plan() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/config/tests/config.rs",
        "#[test]\nfn config_reload() {}\n",
    );
    project.write(
        "upstream/watchexec/crates/ignore-files/tests/ignore.rs",
        "#[test]\nfn ignore_match() {}\n",
    );
    project.write("upstream/watchexec/examples/restart.rs", "fn main() {}\n");
    project.write("upstream/watchexec/examples/debounce.rs", "fn main() {}\n");
    project.write("upstream/watchexec/fixtures/save.json", "{}\n");
    project.write("upstream/watchexec/fixtures/rename.json", "{}\n");
    project.write("upstream/watchexec/fixtures/delete.json", "{}\n");
    project.write("upstream/watchexec/fixtures/signals.json", "{}\n");
    project.write("upstream/watchexec/fixtures/config.toml", "debounce = 50\n");
    project.write("upstream/watchexec/fixtures/logging.json", "{}\n");
    project.write("upstream/watchexec/cliff.toml", "tag_pattern = \"v*\"\n");
    project.write(
        "upstream/watchexec/completions/bash",
        "complete -C watchexec watchexec\n",
    );
    project.write(
        "upstream/watchexec/crates/cli/README.md",
        "Install watchexec with cargo install or packaged completions.\n",
    );
    project.write(
        "upstream/watchexec/install/install.ps1",
        "Write-Output watchexec\n",
    );
    project.write("upstream/watchexec/target/release/watchexec", "binary\n");
    project.write(".kobo/evidence/generated-main.rs", "fn main() {}\n");
    project.write(".kobo/evidence/generated-main.kobo.map", "{}\n");
    write_project_support_proof_marker_test(project);
}

fn write_project_support_proof_marker_test(project: &TestProject) {
    let markers = all_fixture_proof_markers();
    let marker_lines = markers
        .iter()
        .map(|marker| format!("        {marker:?},"))
        .collect::<Vec<_>>()
        .join("\n");
    project.write(
        "upstream/watchexec/crates/supervisor/tests/project_support_proof_markers.rs",
        &format!(
            r#"#[test]
fn project_support_future_proof_markers() {{
    for marker in [
{marker_lines}
    ] {{
        println!("{{marker}}");
    }}
}}
"#
        ),
    );
}

fn all_fixture_proof_markers() -> Vec<String> {
    let mut markers = std::collections::BTreeSet::new();
    for subject in [
        "filesystem_events",
        "watcher_backend",
        "paths",
        "process_execution",
        "signals",
        "process_groups",
        "environment_variables",
        "terminal_io",
        "stdio",
        "timers",
    ] {
        markers.extend(command_proof_markers("platform_model", subject));
    }
    for subject in [
        "watcher_backend",
        "async_runtime",
        "process_handling",
        "signal_handling",
        "ignore_path",
        "config",
        "cli",
        "shell_parsing",
        "serialization",
        "logging_tracing",
        "terminal_helpers",
        "errors",
    ] {
        markers.extend(command_proof_markers("adapter_summary", subject));
    }
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
        "install_behavior",
    ] {
        markers.extend(command_proof_markers("test_release_parity", subject));
    }
    for subject in [
        "startup",
        "steady_state",
        "restart",
        "memory",
        "binary",
        "watch_tree_scaling",
        "event_burst_scaling",
    ] {
        markers.extend(command_proof_markers("performance", subject));
    }
    markers.extend(command_proof_markers("async_runtime", "async_runtime"));
    markers.extend(command_proof_markers("reviewer_report", "reviewer-a"));
    markers.extend(command_proof_markers("reviewer_report", "reviewer-b"));
    markers.into_iter().collect()
}

fn upstream_module_paths() -> &'static [&'static str] {
    &[
        "crates/lib/src/lib.rs",
        "crates/lib/src/watchexec.rs",
        "crates/lib/src/paths.rs",
        "crates/lib/src/late_join_set.rs",
        "crates/lib/src/action/worker.rs",
        "crates/lib/src/action/return.rs",
        "crates/lib/src/sources/fs.rs",
        "crates/lib/src/sources/signal.rs",
        "crates/events/src/lib.rs",
        "crates/events/src/fs.rs",
        "crates/events/src/serde_formats.rs",
        "crates/cli/src/main.rs",
        "crates/cli/src/lib.rs",
        "crates/cli/src/args/logging.rs",
        "crates/cli/src/config.rs",
        "crates/supervisor/src/lib.rs",
        "crates/supervisor/src/command.rs",
        "crates/supervisor/src/errors.rs",
        "crates/supervisor/src/job/state.rs",
        "crates/supervisor/src/policy.rs",
        "crates/supervisor/src/debounce.rs",
        "crates/supervisor/tests/project_support_proof_markers.rs",
        "crates/platform/src/lib.rs",
        "crates/platform/src/windows.rs",
        "crates/platform/src/linux.rs",
        "crates/platform/src/macos.rs",
        "crates/platform/src/polling.rs",
        "crates/signals/src/lib.rs",
        "crates/ignore-files/src/lib.rs",
        "crates/config/src/lib.rs",
        "crates/logging/src/lib.rs",
        "crates/errors/src/lib.rs",
    ]
}

fn proof_debt_modules() -> &'static [&'static str] {
    upstream_module_paths()
}

fn build_evidence_command_transcript(
    project: &TestProject,
    argv: &[&str],
    cwd: Option<&str>,
) -> EvidenceCommandTranscript {
    let command_cwd = cwd
        .map(|path| project.root.join(path))
        .unwrap_or_else(|| project.root.clone());
    let output = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .current_dir(command_cwd)
        .output()
        .expect("evidence command should run");
    assert!(
        output.status.success(),
        "evidence command failed: {}\n{}",
        argv.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    let command = argv.join(" ");
    let source = serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "command": command,
        "status": if output.status.success() { "passed" } else { "failed" },
        "exit_code": output.status.code(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
    .expect("transcript should serialize");
    let hash = stable_hash(&source);
    EvidenceCommandTranscript {
        command,
        argv_json: json_string_array(argv),
        cwd: cwd.map(str::to_owned),
        output_match: if argv.get(1) == Some(&"metadata") {
            "exact"
        } else {
            "exit_code"
        },
        source,
        hash,
    }
}

fn json_string_array(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| serde_json::to_string(value).expect("test string should serialize"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn owned_json_string_array(values: &[String]) -> String {
    values
        .iter()
        .map(|value| serde_json::to_string(value).expect("test string should serialize"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn subject_covered_paths(evidence_kind: &str, subject: &str) -> &'static [&'static str] {
    match (evidence_kind, subject) {
        ("language_surface", _) => upstream_module_paths(),
        ("platform_model", "filesystem_events") => &[
            "crates/events/src/fs.rs",
            "crates/lib/src/sources/fs.rs",
            "crates/lib/src/watchexec.rs",
        ],
        ("platform_model", "watcher_backend") => &[
            "crates/lib/src/watchexec.rs",
            "crates/lib/src/sources/fs.rs",
        ],
        ("platform_model", "paths") => &["crates/lib/src/paths.rs"],
        ("platform_model", "process_execution") => &[
            "crates/supervisor/src/command.rs",
            "crates/lib/src/action/worker.rs",
        ],
        ("platform_model", "signals") => &[
            "crates/signals/src/lib.rs",
            "crates/lib/src/sources/signal.rs",
        ],
        ("platform_model", "process_groups") => &["crates/supervisor/src/command.rs"],
        ("platform_model", "environment_variables") => &["crates/cli/src/config.rs"],
        ("platform_model", "terminal_io") | ("platform_model", "stdio") => {
            &["crates/cli/src/lib.rs"]
        }
        ("platform_model", "timers") => &[
            "crates/lib/src/action/worker.rs",
            "crates/cli/src/config.rs",
        ],
        ("adapter_summary", "watcher_backend") => &["crates/lib/src/sources/fs.rs"],
        ("adapter_summary", "async_runtime") => &["crates/lib/src/late_join_set.rs"],
        ("adapter_summary", "process_handling") => &["crates/supervisor/src/command.rs"],
        ("adapter_summary", "signal_handling") => &["crates/signals/src/lib.rs"],
        ("adapter_summary", "ignore_path") => &["crates/ignore-files/src/lib.rs"],
        ("adapter_summary", "config") => &["crates/cli/src/config.rs"],
        ("adapter_summary", "cli") => &["crates/cli/src/args/logging.rs"],
        ("adapter_summary", "shell_parsing") => &["crates/supervisor/src/command.rs"],
        ("adapter_summary", "serialization") => &["crates/events/src/serde_formats.rs"],
        ("adapter_summary", "logging_tracing") => &["crates/cli/src/args/logging.rs"],
        ("adapter_summary", "terminal_helpers") => &["crates/cli/src/lib.rs"],
        ("adapter_summary", "errors") => &["crates/supervisor/src/errors.rs"],
        ("async_runtime", _) => &[
            "crates/lib/src/late_join_set.rs",
            "crates/lib/src/action/worker.rs",
        ],
        ("generated_backend", _) => upstream_module_paths(),
        ("test_release_parity", "upstream_tests") | ("upstream_tests", _) => &[
            "crates/cli/tests/cli_flags.rs",
            "crates/supervisor/tests/restart.rs",
        ],
        ("test_release_parity", "kobo_replay_tests") => &["fixtures/save.json"],
        ("test_release_parity", "kobo_liveness_tests") => &["crates/supervisor/src/job/state.rs"],
        ("test_release_parity", "cli_behavior") => &["crates/cli/src/main.rs"],
        ("test_release_parity", "config_behavior") => &["crates/cli/src/config.rs"],
        ("test_release_parity", "exit_behavior") => &["crates/lib/src/action/return.rs"],
        ("test_release_parity", "logging_behavior") => &["crates/cli/src/args/logging.rs"],
        ("test_release_parity", "package_behavior") => &["Cargo.toml", "cliff.toml"],
        ("test_release_parity", "install_behavior") => {
            &["crates/cli/Cargo.toml", "completions/bash"]
        }
        ("test_release_parity", "platform_behavior") => {
            &["crates/lib/src/paths.rs", "crates/lib/src/sources/fs.rs"]
        }
        ("performance", "startup")
        | ("performance", "steady_state")
        | ("performance", "restart") => &["crates/lib/src/watchexec.rs"],
        ("performance", "memory") | ("performance", "binary") => &[
            "Cargo.toml",
            "crates/cli/Cargo.toml",
            "target/release/watchexec",
        ],
        ("performance", "watch_tree_scaling") | ("performance", "event_burst_scaling") => {
            &["crates/lib/src/sources/fs.rs"]
        }
        _ => upstream_module_paths(),
    }
}

fn command_proves_target(evidence_kind: &str, subject: &str) -> String {
    match evidence_kind {
        "upstream_inventory" => "upstream_inventory".to_owned(),
        "language_surface" => "language_surface".to_owned(),
        "generated_backend" => "generated_backend".to_owned(),
        "async_runtime" => "async_runtime".to_owned(),
        "upstream_tests" => "upstream_tests".to_owned(),
        "platform_model"
        | "adapter_summary"
        | "test_release_parity"
        | "performance"
        | "proof_debt_report"
        | "reviewer_report" => subject.to_owned(),
        _ => subject.to_owned(),
    }
}

fn command_proof_markers(evidence_kind: &str, subject: &str) -> Vec<String> {
    match (evidence_kind, subject) {
        ("async_runtime", "async_runtime") => string_vec(&[
            "kobo-proof:async_runtime:spawn",
            "kobo-proof:async_runtime:join",
            "kobo-proof:async_runtime:cancel",
            "kobo-proof:async_runtime:select",
            "kobo-proof:async_runtime:timer",
            "kobo-proof:async_runtime:channel",
            "kobo-proof:async_runtime:backpressure",
            "kobo-proof:async_runtime:shutdown",
            "kobo-proof:async_runtime:blocking",
        ]),
        ("upstream_tests", _) | ("test_release_parity", "upstream_tests") => Vec::new(),
        ("test_release_parity", "kobo_replay_tests") => {
            string_vec(&["kobo-proof:kobo_replay_tests:replay"])
        }
        ("test_release_parity", "kobo_liveness_tests") => {
            string_vec(&["kobo-proof:kobo_liveness_tests:liveness"])
        }
        ("test_release_parity", "cli_behavior") => string_vec(&["kobo-proof:cli_behavior:cli"]),
        ("test_release_parity", "config_behavior") => {
            string_vec(&["kobo-proof:config_behavior:config"])
        }
        ("test_release_parity", "exit_behavior") => string_vec(&["kobo-proof:exit_behavior:exit"]),
        ("test_release_parity", "logging_behavior") => {
            string_vec(&["kobo-proof:logging_behavior:logging"])
        }
        ("test_release_parity", "package_behavior") => {
            string_vec(&["kobo-proof:package_behavior:package"])
        }
        ("test_release_parity", "platform_behavior") => {
            string_vec(&["kobo-proof:platform_behavior:platform"])
        }
        ("test_release_parity", "install_behavior") => {
            string_vec(&["kobo-proof:install_behavior:install"])
        }
        ("performance", "startup") => string_vec(&["kobo-proof:startup:measurement"]),
        ("performance", "steady_state") => string_vec(&["kobo-proof:steady_state:measurement"]),
        ("performance", "restart") => string_vec(&["kobo-proof:restart:measurement"]),
        ("performance", "memory") => string_vec(&["kobo-proof:memory:measurement"]),
        ("performance", "binary") => string_vec(&["kobo-proof:binary:measurement"]),
        ("performance", "watch_tree_scaling") => {
            string_vec(&["kobo-proof:watch_tree_scaling:measurement"])
        }
        ("performance", "event_burst_scaling") => {
            string_vec(&["kobo-proof:event_burst_scaling:measurement"])
        }
        ("reviewer_report", "reviewer-a") => {
            string_vec(&["kobo-proof:reviewer-a:independent-review"])
        }
        ("reviewer_report", "reviewer-b") => {
            string_vec(&["kobo-proof:reviewer-b:independent-review"])
        }
        ("adapter_summary", "async_runtime") => string_vec(&[
            "kobo-proof:adapter_summary:async_runtime",
            "kobo-proof:async_runtime:cancel",
            "kobo-proof:async_runtime:timer",
        ]),
        ("adapter_summary", subject) => {
            string_vec(&[&format!("kobo-proof:adapter_summary:{subject}")])
        }
        ("platform_model", subject) => {
            string_vec(&[&format!("kobo-proof:platform_model:{subject}")])
        }
        _ => Vec::new(),
    }
}

fn string_vec(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn measurements_json(evidence_kind: &str, subject: &str) -> String {
    let measurement = if evidence_kind == "performance" {
        serde_json::json!([{
            "name": subject,
            "value": performance_value(subject),
            "baseline_value": performance_baseline(subject),
            "sample_count": 5,
            "unit": performance_unit(subject),
        }])
    } else {
        serde_json::json!([{
            "name": subject,
            "value": 1.0,
            "unit": "sample",
        }])
    };
    measurement.to_string()
}

fn performance_unit(subject: &str) -> &'static str {
    match subject {
        "startup" | "steady_state" | "restart" => "ms",
        "memory" => "MiB",
        "binary" => "bytes",
        "watch_tree_scaling" => "paths_per_second",
        "event_burst_scaling" => "events_per_second",
        _ => "sample",
    }
}

fn performance_value(subject: &str) -> f64 {
    match subject {
        "startup" => 42.0,
        "steady_state" => 3.5,
        "restart" => 18.0,
        "memory" => 28.0,
        "binary" => 8_388_608.0,
        "watch_tree_scaling" => 12_000.0,
        "event_burst_scaling" => 80_000.0,
        _ => 1.0,
    }
}

fn performance_baseline(subject: &str) -> f64 {
    match subject {
        "startup" => 50.0,
        "steady_state" => 4.0,
        "restart" => 20.0,
        "memory" => 32.0,
        "binary" => 9_437_184.0,
        "watch_tree_scaling" => 10_000.0,
        "event_burst_scaling" => 75_000.0,
        _ => 1.0,
    }
}

fn write_evidence(
    project: &TestProject,
    path: &str,
    evidence_kind: &str,
    subject: &str,
    evidence_command: &EvidenceCommandTranscript,
) {
    let transcript_path = path.replace(".json", ".transcript.json");
    project.write(&transcript_path, &evidence_command.source);
    let command_json =
        serde_json::to_string(&evidence_command.command).expect("test command should serialize");
    let cwd_json = evidence_command
        .cwd
        .as_ref()
        .map(|cwd| format!(r#", "cwd": {}"#, serde_json::to_string(cwd).unwrap()))
        .unwrap_or_default();
    let output_match_json = format!(
        r#", "output_match": {}"#,
        serde_json::to_string(evidence_command.output_match).unwrap()
    );
    let proves_target = command_proves_target(evidence_kind, subject);
    let proof_markers = command_proof_markers(evidence_kind, subject);
    let proof_markers_json = owned_json_string_array(&proof_markers);
    let covered_paths = json_string_array(subject_covered_paths(evidence_kind, subject));
    let covered_modules = json_string_array(proof_debt_modules());
    let measurements_json = measurements_json(evidence_kind, subject);
    let future_fields = future_fields_json(evidence_kind, subject, &evidence_command.hash);
    let reviewer_fields = reviewer_fields_json(evidence_kind, subject, &evidence_command.hash);
    let decision = if evidence_kind == "proof_debt_report" {
        "same_project_map"
    } else {
        "equivalent_or_stronger"
    };
    project.write(
        path,
        &format!(
            r#"{{
  "schema_version": 1,
  "evidence_kind": "{evidence_kind}",
  "subject": "{subject}",
  "source_revision": "test-upstream-fixture",
  "upstream_root": "upstream/watchexec",
  "checks": ["parse", "check", "lower", "source_map", "rare_diagnostics"],
  "covered_paths": [{covered_paths}],
  "covered_modules": [{covered_modules}],
  "tested_platforms": ["windows", "macos", "linux"],
  "commands": [
    {{"command": {command_json}, "argv": [{argv_json}], "status": "passed", "transcript_path": "{transcript_path}", "output_hash": "{transcript_hash}", "proves": ["{proves_target}"], "proof_markers": [{proof_markers_json}]{cwd_json}{output_match_json}}}
  ],
  "behavior_tests": ["watcher", "restart", "signal", "stdin", "path_filter"],
  "conformance_results": [
    {{"name": "{subject}-conformance", "status": "passed"}}
  ],
  "stale_check": {{"status": "passed", "crate_version": "1.0.0", "features": ["default", "polling", "rt", "time", "derive"]}},
  "measurements": {measurements_json},
  "mutation_results": ["task-order", "timer-order", "cancel-order", "channel-delivery"],
  "scheduler_facts": ["watcher-batching", "restart-ordering", "signal-delivery", "child-exit-race"],
  "decision": "{decision}"{future_fields}{reviewer_fields}
}}"#
            ,
            argv_json = evidence_command.argv_json.as_str(),
            transcript_hash = evidence_command.hash.as_str(),
        ),
    );
}

fn future_fields_json(evidence_kind: &str, subject: &str, command_hash: &str) -> String {
    match evidence_kind {
        "language_surface" => r#",
  "source_kind": "kobo_whole_project",
  "kobo_owned_modules": ["src/supervisor.kobo", "examples/restart.kobo"]"#
            .to_owned(),
        "platform_model" => r#",
  "platform_observations": [
    {"platform": "windows", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "artifact_path": ".kobo/evidence/platform-windows.json"},
    {"platform": "macos", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "artifact_path": ".kobo/evidence/platform-macos.json"},
    {"platform": "linux", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "artifact_path": ".kobo/evidence/platform-linux.json"}
  ]"#
        .to_owned(),
        "adapter_summary" => format!(
            r#",
  "adapter_name": "{}",
  "adapter_version": "1.0.0",
  "conformance_results": [{{"adapter_kind": "{}", "status": "passed", "command_output_hash": "{}"}}]"#,
            subject, subject, command_hash
        ),
        "async_runtime" => r#",
  "runtime_semantics": ["spawn", "join", "cancel", "select", "timer", "channel", "backpressure", "shutdown", "blocking"]"#
            .to_owned(),
        "generated_backend" => r#",
  "generated_artifacts": [{"path": ".kobo/evidence/generated-main.rs", "source_map": ".kobo/evidence/generated-main.kobo.map"}],
  "debug_workflows": ["inspect_clean_cargo", "cargo_check", "source_map_diagnostics", "replay_debug"]"#
            .to_owned(),
        "test_release_parity" => {
            let install_artifacts = if subject == "install_behavior" {
                r#",
  "install_artifacts": ["upstream/watchexec/completions/bash"]"#
            } else {
                ""
            };
            let replacement = if subject == "upstream_tests" {
                r#",
  "replacement_root": "upstream/watchexec",
  "replacement_source": "kobo_generated""#
            } else {
                ""
            };
            format!(
                r#",
  "parity_results": [{{"name": "{}", "status": "passed", "command_output_hash": "{}"}}]{}{}"#,
                subject, command_hash, install_artifacts, replacement
            )
        }
        "performance" => r#",
  "measurement_source": "bench_run""#
            .to_owned(),
        "upstream_tests" => r#",
  "replacement_root": "upstream/watchexec",
  "replacement_source": "kobo_generated""#
            .to_owned(),
        _ => String::new(),
    }
}

fn reviewer_fields_json(evidence_kind: &str, reviewer: &str, command_hash: &str) -> String {
    if evidence_kind != "reviewer_report" {
        return String::new();
    }
    let review_sections = reviewer_section_names()
        .iter()
        .map(|section| {
            serde_json::json!({
                "name": section,
                "decision": "equivalent_or_stronger",
                "boundary": "honest",
                "evidence_refs": ["commands", "covered_paths", "project-support.json"],
                "command_output_hash": command_hash,
            })
        })
        .collect::<Vec<_>>();
    format!(
        r#",
  "reviewer": {},
  "boundary_decision": "honest_boundary",
  "comparison_summary": "all required comparisons approve equivalent or stronger behavior",
  "review_sections": {}"#,
        serde_json::to_string(reviewer).expect("reviewer should serialize"),
        serde_json::to_string(&review_sections).expect("review sections should serialize")
    )
}

fn reviewer_section_names() -> &'static [&'static str] {
    &[
        "behavior",
        "generated_rust",
        "diagnostics",
        "proof_debt",
        "real_command_output",
        "release_artifacts",
    ]
}

fn write_complete_support_manifest(project: &TestProject) {
    project.write(
        ".kobo/project-support.json",
        r#"{
  "schema_version": 1,
  "claim": "project_support",
  "upstream_inventory": {
    "project_name": "watchexec",
    "source_revision": "test-upstream-fixture",
    "root": "upstream/watchexec",
    "workspace_manifest": "Cargo.toml",
    "crate_tree": [
      "crates/lib/src/lib.rs",
      "crates/events/src/lib.rs",
      "crates/cli/src/main.rs",
      "crates/cli/src/lib.rs",
      "crates/supervisor/src/lib.rs",
      "crates/supervisor/src/policy.rs",
      "crates/supervisor/src/debounce.rs",
      "crates/supervisor/tests/project_support_proof_markers.rs",
      "crates/platform/src/lib.rs",
      "crates/platform/src/windows.rs",
      "crates/platform/src/linux.rs",
      "crates/platform/src/macos.rs",
      "crates/platform/src/polling.rs",
      "crates/signals/src/lib.rs",
      "crates/ignore-files/src/lib.rs",
      "crates/config/src/lib.rs",
      "crates/logging/src/lib.rs",
      "crates/errors/src/lib.rs"
    ],
    "modules": [
      "crates/lib/src/lib.rs",
      "crates/events/src/lib.rs",
      "crates/cli/src/main.rs",
      "crates/cli/src/lib.rs",
      "crates/supervisor/src/lib.rs",
      "crates/supervisor/src/policy.rs",
      "crates/supervisor/src/debounce.rs",
      "crates/supervisor/tests/project_support_proof_markers.rs",
      "crates/platform/src/lib.rs",
      "crates/platform/src/windows.rs",
      "crates/platform/src/linux.rs",
      "crates/platform/src/macos.rs",
      "crates/platform/src/polling.rs",
      "crates/signals/src/lib.rs",
      "crates/ignore-files/src/lib.rs",
      "crates/config/src/lib.rs",
      "crates/logging/src/lib.rs",
      "crates/errors/src/lib.rs"
    ],
    "public_types": ["Watchexec", "Event", "CliSurface", "Supervisor", "RestartPolicy", "WatchEvent", "DebounceWindow", "PlatformModel", "SignalPlan", "IgnoreMatcher", "ConfigSource", "LogEvent", "ErrorReport"],
    "cli_surfaces": ["watchexec --restart", "watchexec --watch", "watchexec --signal", "watchexec --on-busy-update", "watchexec --debounce", "watchexec --print-events"],
    "test_fixtures": [
      "crates/cli/tests/cli_flags.rs",
      "crates/supervisor/tests/restart.rs",
      "crates/supervisor/tests/project_support_proof_markers.rs",
      "crates/platform/tests/platform.rs",
      "crates/signals/tests/signals.rs",
      "crates/config/tests/config.rs",
      "crates/ignore-files/tests/ignore.rs",
      "fixtures/save.json",
      "fixtures/rename.json",
      "fixtures/delete.json",
      "fixtures/signals.json",
      "fixtures/config.toml",
      "fixtures/logging.json"
    ],
    "platform_paths": [
      {"path": "crates/platform/src/windows.rs", "disposition": "formal_adapter", "correctness_relevant": true, "source_preserved": true},
      {"path": "crates/platform/src/linux.rs", "disposition": "formal_adapter", "correctness_relevant": true, "source_preserved": true},
      {"path": "crates/platform/src/macos.rs", "disposition": "formal_adapter", "correctness_relevant": true, "source_preserved": true},
      {"path": "crates/platform/src/polling.rs", "disposition": "formal_adapter", "correctness_relevant": true, "source_preserved": true}
    ],
    "feature_combinations": [["default"], ["default", "polling"], ["default", "signals"], ["serde"]],
    "examples": ["examples/restart.rs", "examples/debounce.rs"],
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
    "feature_matrix": [["default"], ["default", "polling"], ["default", "signals"], ["serde"]],
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
    {"kind": "shell_parsing", "name": "kobo-shell-boundary", "crate_source": {"kind": "project_module", "path": "src/adapters/process.rs"}, "version_range": "project", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["shell-wrap", "no-shell"], "unsupported_guarantees": ["host-shell-parser-equivalence"], "conformance_tests": ["quoted-command"], "replay_evidence": ".kobo/evidence/adapter-shell.json", "stale_summary_detection": true},
    {"kind": "serialization", "name": "serde", "crate_source": {"kind": "cargo_dependency", "name": "serde"}, "version_range": "^1", "cargo_features": ["derive"], "summary_version": 1, "modeled_facts": ["json"], "unsupported_guarantees": ["format-autodetect"], "conformance_tests": ["witness-json"], "replay_evidence": ".kobo/evidence/adapter-serialization.json", "stale_summary_detection": true},
    {"kind": "logging_tracing", "name": "tracing", "crate_source": {"kind": "cargo_dependency", "name": "tracing"}, "version_range": "^0.1", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["events"], "unsupported_guarantees": ["terminal-color-equivalence"], "conformance_tests": ["log-routing"], "replay_evidence": ".kobo/evidence/adapter-logging.json", "stale_summary_detection": true},
    {"kind": "terminal_helpers", "name": "kobo-terminal-boundary", "crate_source": {"kind": "std", "name": "std::io"}, "version_range": "std", "cargo_features": ["default"], "summary_version": 1, "modeled_facts": ["tty", "inherited-handles"], "unsupported_guarantees": ["terminal-emulator-equivalence"], "conformance_tests": ["stdio-forwarding"], "replay_evidence": ".kobo/evidence/adapter-terminal.json", "stale_summary_detection": true},
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
    "install_behavior": ".kobo/evidence/parity-install_behavior.json",
    "performance": {
      "startup": ".kobo/evidence/perf-startup.json",
      "steady_state": ".kobo/evidence/perf-steady_state.json",
      "restart": ".kobo/evidence/perf-restart.json",
      "memory": ".kobo/evidence/perf-memory.json",
      "binary": ".kobo/evidence/perf-binary.json",
      "watch_tree_scaling": ".kobo/evidence/perf-watch_tree_scaling.json",
      "event_burst_scaling": ".kobo/evidence/perf-event_burst_scaling.json"
    },
    "release_artifacts": [".kobo/evidence/release.zip"]
  },
  "proof_debt_map": [
    {"module": "crates/lib/src/lib.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/watchexec.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/paths.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/late_join_set.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/action/worker.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/action/return.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/sources/fs.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/lib/src/sources/signal.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/events/src/lib.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/events/src/fs.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/events/src/serde_formats.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/cli/src/main.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/cli/src/lib.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/cli/src/args/logging.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/cli/src/config.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/src/lib.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/src/command.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/src/errors.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/src/job/state.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/src/policy.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/src/debounce.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/supervisor/tests/project_support_proof_markers.rs", "classification": "sampled", "criticality": "non-critical", "release_blocking": false, "justification": "test harness evidence only"},
    {"module": "crates/platform/src/lib.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/platform/src/windows.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/platform/src/linux.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/platform/src/macos.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/platform/src/polling.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/signals/src/lib.rs", "classification": "modeled", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/ignore-files/src/lib.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/config/src/lib.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/logging/src/lib.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false},
    {"module": "crates/errors/src/lib.rs", "classification": "adapter-backed", "criticality": "correctness-critical", "release_blocking": false}
  ],
  "proof_debt_map_reports": {
    "debt_summary": ".kobo/evidence/proof-debt-debt_summary.json",
    "proof_report": ".kobo/evidence/proof-debt-proof_report.json",
    "replay_report": ".kobo/evidence/proof-debt-replay_report.json",
    "inspect_output": ".kobo/evidence/proof-debt-inspect_output.json"
  },
  "independent_equivalence": {
    "original_upstream_tests": ".kobo/evidence/upstream-tests.json",
    "mutation_tests": ["watcher", "child", "cancellation", "config", "ignore_rules", "async_scheduling", "platform", "generated_backend_output"],
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
        value["project_support"]["status"], "ready",
        "ready project blockers: {}",
        value["project_support"]["blockers"]
    );
    assert_eq!(value["project_support"]["claim"], "project_support");
    assert_eq!(
        value["project_support"]["replacement_claim"]["full_rewrite"], "blocked",
        "project support must not imply a full rewrite claim"
    );
    assert_eq!(
        value["project_support"]["replacement_claim"]["clean_replacement"], "blocked",
        "project support must not imply a clean replacement claim"
    );
    assert_contains(
        &value["project_support"]["replacement_claim"].to_string(),
        "supports the project",
        "claim posture should say Kobo supports the project without being designed for dogfood",
    );
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
fn project_map_exposes_whole_project_coverage_from_support_manifest() {
    let project = TestProject::new("project-map-whole-project-coverage");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let main = project.root.join("src/main.kobo");

    let debt = run_kobo(&[s("debt"), s("--json"), path_arg(&main)], &project.root);
    assert_success(&debt, "debt JSON should include whole-project coverage");
    let value = parse_stdout_json(&debt);
    let coverage = &value["project_map"]["whole_project_coverage"];
    assert_eq!(coverage["status"], "evidence_present");
    assert_eq!(coverage["feature_combinations"], 4);
    assert_eq!(coverage["upstream_inventory_counts"]["crate_tree"], 18);
    assert_eq!(coverage["upstream_inventory_counts"]["platform_paths"], 4);
    assert_eq!(coverage["language_flags"]["parse"], true);
    assert_eq!(
        coverage["adapter_decisions"]["watcher_backend"],
        "formal_adapter"
    );
    assert_eq!(
        coverage["adapter_decisions"]["signal_handling"],
        "kobo_owned_adapter"
    );
    assert_eq!(coverage["platform_model_grades"]["terminal_io"], "sampled");
    assert_eq!(coverage["generated_backend_flags"]["source_mapped"], true);
    assert_eq!(coverage["test_release_parity_counts"]["performance"], 7);
    assert_eq!(coverage["test_release_parity_counts"]["release_artifacts"], 1);
    assert_eq!(coverage["proof_debt_modules"], 32);
    assert_eq!(coverage["mutation_tests"], 8);
    assert_eq!(coverage["reviewer_reports"], 2);
    assert_eq!(coverage["release_gate"], "evidence_visible");

    let inspect = run_kobo(&[s("inspect"), path_arg(&main)], &project.root);
    assert_success(&inspect, "inspect should include the expanded project map");
    assert_contains(
        &inspect.stdout,
        "coverage=evidence_present",
        "inspect should expose the same whole-project coverage status as debt",
    );
}

#[test]
fn release_profile_blocks_generated_backend_support_debt() {
    let project = TestProject::new("release-project-support-generated-backend");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(r#""source_mapped": true"#, r#""source_mapped": false"#),
    )
    .expect("support manifest should write");

    assert_release_project_support_gate_blocks(
        &project,
        "generated backend release gate missing source_mapped",
    );
}

#[test]
fn release_profile_blocks_async_runtime_support_debt() {
    let project = TestProject::new("release-project-support-async");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(r#""channel-delivery""#, r#""channel-delivery-missing""#),
    )
    .expect("support manifest should write");

    assert_release_project_support_gate_blocks(
        &project,
        "async runtime release gate missing channel-delivery",
    );
}

#[test]
fn release_profile_blocks_test_release_parity_support_debt() {
    let project = TestProject::new("release-project-support-parity");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""release_artifacts": [".kobo/evidence/release.zip"]"#,
            r#""release_artifacts": []"#,
        ),
    )
    .expect("support manifest should write");

    assert_release_project_support_gate_blocks(
        &project,
        "test release parity gate missing release artifacts",
    );
}

#[test]
fn release_profile_blocks_proof_debt_support_debt() {
    let project = TestProject::new("release-project-support-proof-debt");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#"{"module": "crates/supervisor/src/command.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false}"#,
            r#"{"module": "crates/supervisor/src/command.rs", "classification": "debt", "criticality": "correctness-critical", "release_blocking": true}"#,
        ),
    )
    .expect("support manifest should write");

    assert_release_project_support_gate_blocks(
        &project,
        "proof debt release gate blocked crates/supervisor/src/command.rs",
    );
}

fn assert_release_project_support_gate_blocks(project: &TestProject, expected: &str) {
    let main = project.root.join("src/main.kobo");
    for command in ["check", "build"] {
        let output = run_kobo(
            &[s(command), s("--profile"), s("release"), path_arg(&main)],
            &project.root,
        );
        assert_failure(
            &output,
            &format!("release {command} should block project support debt"),
        );
        assert_contains(
            &output.combined(),
            "release project support gate",
            "release profile should name the project support gate",
        );
        assert_contains(
            &output.combined(),
            expected,
            "release project support gate should name the blocking evidence",
        );
    }
}

#[test]
fn doctor_project_support_rejects_replacement_claim_overreach() {
    let project = TestProject::new("doctor-project-support-claim-overreach");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""claim": "project_support","#,
            r#""claim": "project_support",
  "replacement_claim": {
    "full_rewrite": "ready",
    "clean_replacement": "ready"
  },"#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "overclaim report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_eq!(
        value["project_support"]["replacement_claim"]["clean_replacement"], "blocked",
        "report posture stays conservative even when the manifest overclaims"
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "project support full rewrite claim must remain blocked",
        "project support should block full rewrite overclaims",
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "project support clean replacement claim must remain blocked",
        "project support should block clean replacement overclaims",
    );
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
                r#"{"module": "crates/supervisor/src/command.rs", "classification": "proved", "criticality": "correctness-critical", "release_blocking": false}"#,
                r#"{"module": "crates/supervisor/src/command.rs", "classification": "debt", "criticality": "correctness-critical", "release_blocking": false}"#,
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
        "critical module crates/supervisor/src/command.rs is classified as debt",
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
            r#""feature_matrix": [["default"], ["default", "polling"], ["default", "signals"], ["serde"]]"#,
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
fn doctor_project_support_rejects_metadata_only_future_evidence() {
    let project = TestProject::new("doctor-project-support-metadata-only-future");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let inventory_evidence_path = project.root.join(".kobo/evidence/upstream-inventory.json");
    let async_evidence_path = project.root.join(".kobo/evidence/async.json");
    let inventory_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&inventory_evidence_path).expect("inventory evidence should read"),
    )
    .expect("inventory evidence should parse");
    let mut async_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&async_evidence_path).expect("async evidence should read"),
    )
    .expect("async evidence should parse");
    async_evidence["commands"] = inventory_evidence["commands"].clone();
    fs::write(
        &async_evidence_path,
        serde_json::to_string_pretty(&async_evidence).expect("async evidence should serialize"),
    )
    .expect("async evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "metadata-only future evidence report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "conformance_evidence evidence command does not prove async_runtime",
        "future evidence must be backed by a subject-specific command",
    );
}

#[test]
fn doctor_project_support_rejects_unrelated_subject_coverage() {
    let project = TestProject::new("doctor-project-support-unrelated-subject-coverage");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let adapter_evidence_path = project.root.join(".kobo/evidence/adapter-async.json");
    let mut adapter_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&adapter_evidence_path).expect("adapter evidence should read"),
    )
    .expect("adapter evidence should parse");
    adapter_evidence["covered_paths"] = serde_json::json!(["crates/cli/src/main.rs"]);
    fs::write(
        &adapter_evidence_path,
        serde_json::to_string_pretty(&adapter_evidence).expect("adapter evidence should serialize"),
    )
    .expect("adapter evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "unrelated subject coverage report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "replay_evidence evidence does not cover async_runtime subject path",
        "adapter evidence must cover paths tied to its subject",
    );
}

#[test]
fn doctor_project_support_rejects_missing_covered_source_path() {
    let project = TestProject::new("doctor-project-support-missing-covered-path");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let evidence_path = project.root.join(".kobo/evidence/language.json");
    let mut evidence: Value =
        serde_json::from_str(&fs::read_to_string(&evidence_path).expect("evidence should read"))
            .expect("evidence should parse");
    evidence["covered_paths"][0] = serde_json::json!("crates/lib/src/missing.rs");
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&evidence).expect("evidence should serialize"),
    )
    .expect("evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "missing covered path report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "evidence_path evidence covered path does not exist: crates/lib/src/missing.rs",
        "evidence coverage should be tied to real source files",
    );
}

#[test]
fn doctor_project_support_rejects_command_without_subject_proof_marker() {
    let project = TestProject::new("doctor-project-support-missing-proof-marker");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let async_evidence_path = project.root.join(".kobo/evidence/async.json");
    let mut async_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&async_evidence_path).expect("async evidence should read"),
    )
    .expect("async evidence should parse");
    async_evidence["commands"][0]["proves"] = serde_json::json!(["watcher_backend"]);
    fs::write(
        &async_evidence_path,
        serde_json::to_string_pretty(&async_evidence).expect("async evidence should serialize"),
    )
    .expect("async evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "missing proof marker report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "conformance_evidence evidence command missing proves entry for async_runtime",
        "evidence command must declare the exact subject it proves",
    );
}

#[test]
fn doctor_project_support_rejects_unbound_transcript_proof_marker() {
    let project = TestProject::new("doctor-project-support-unbound-transcript-proof");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let async_evidence_path = project.root.join(".kobo/evidence/async.json");
    let mut async_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&async_evidence_path).expect("async evidence should read"),
    )
    .expect("async evidence should parse");
    let transcript_path = project.root.join(
        async_evidence["commands"][0]["transcript_path"]
            .as_str()
            .unwrap(),
    );
    let transcript_source =
        fs::read_to_string(&transcript_path).expect("async transcript should read");
    let edited_transcript = transcript_source.replace(
        "kobo-proof:async_runtime:channel",
        "removed-proof:async_runtime:channel",
    );
    fs::write(&transcript_path, &edited_transcript).expect("async transcript should write");
    async_evidence["commands"][0]["output_hash"] =
        serde_json::json!(stable_hash(&edited_transcript));
    fs::write(
        &async_evidence_path,
        serde_json::to_string_pretty(&async_evidence).expect("async evidence should serialize"),
    )
    .expect("async evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "unbound transcript proof report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "conformance_evidence evidence transcript missing proof marker kobo-proof:async_runtime:channel",
        "evidence proof markers must be present in the transcript artifact",
    );
}

#[test]
fn doctor_project_support_rejects_missing_evidence_path_prepend() {
    let project = TestProject::new("doctor-project-support-missing-path-prepend");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let async_evidence_path = project.root.join(".kobo/evidence/async.json");
    let mut async_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&async_evidence_path).expect("async evidence should read"),
    )
    .expect("async evidence should parse");
    async_evidence["commands"][0]["env_path_prepend"] =
        serde_json::json!(".kobo/evidence/missing-bin");
    fs::write(
        &async_evidence_path,
        serde_json::to_string_pretty(&async_evidence).expect("async evidence should serialize"),
    )
    .expect("async evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "missing-path-prepend report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "conformance_evidence evidence command env_path_prepend does not exist",
        "evidence command environment support must be declared and present",
    );
}

#[test]
fn doctor_project_support_rejects_invalid_evidence_timeout() {
    let project = TestProject::new("doctor-project-support-invalid-timeout");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let async_evidence_path = project.root.join(".kobo/evidence/async.json");
    let mut async_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&async_evidence_path).expect("async evidence should read"),
    )
    .expect("async evidence should parse");
    async_evidence["commands"][0]["timeout_seconds"] = serde_json::json!(0);
    fs::write(
        &async_evidence_path,
        serde_json::to_string_pretty(&async_evidence).expect("async evidence should serialize"),
    )
    .expect("async evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "invalid-timeout report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "conformance_evidence evidence command timeout_seconds must be between 1 and 1800",
        "evidence command timeout must stay bounded",
    );
}

#[test]
fn doctor_project_support_rejects_placeholder_performance_measurement() {
    let project = TestProject::new("doctor-project-support-placeholder-performance");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let performance_evidence_path = project.root.join(".kobo/evidence/perf-startup.json");
    let mut performance_evidence: Value = serde_json::from_str(
        &fs::read_to_string(&performance_evidence_path).expect("performance evidence should read"),
    )
    .expect("performance evidence should parse");
    performance_evidence["measurements"][0]["unit"] = serde_json::json!("passed_command");
    fs::write(
        &performance_evidence_path,
        serde_json::to_string_pretty(&performance_evidence)
            .expect("performance evidence should serialize"),
    )
    .expect("performance evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "placeholder performance report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "startup performance measurement unit passed_command does not prove startup",
        "performance evidence must use a subject-specific measurement unit",
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
        manifest.replace("      \"crates/supervisor/src/lib.rs\",\n", ""),
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
fn doctor_project_support_rejects_filtered_upstream_test_command() {
    let project = TestProject::new("doctor-project-support-filtered-upstream-tests");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let evidence_path = project.root.join(".kobo/evidence/upstream-tests.json");
    let mut evidence: Value =
        serde_json::from_str(&fs::read_to_string(&evidence_path).expect("evidence should read"))
            .expect("evidence should parse");
    let filtered_argv = [
        "cargo",
        "test",
        "--workspace",
        "--tests",
        "--",
        "--nocapture",
    ];
    let filtered_command = filtered_argv.join(" ");
    let transcript_path = project.root.join(
        evidence["commands"][0]["transcript_path"]
            .as_str()
            .expect("transcript path should exist"),
    );
    let mut transcript: Value = serde_json::from_str(
        &fs::read_to_string(&transcript_path).expect("transcript should read"),
    )
    .expect("transcript should parse");
    transcript["command"] = serde_json::json!(filtered_command);
    let transcript_source =
        serde_json::to_string_pretty(&transcript).expect("transcript should serialize");
    fs::write(&transcript_path, &transcript_source).expect("transcript should write");
    evidence["commands"][0]["command"] = serde_json::json!(filtered_command);
    evidence["commands"][0]["argv"] = serde_json::json!(filtered_argv);
    evidence["commands"][0]["proof_markers"] = serde_json::json!([]);
    evidence["commands"][0]["output_hash"] = serde_json::json!(stable_hash(&transcript_source));
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&evidence).expect("evidence should serialize"),
    )
    .expect("evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "filtered-upstream report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "original_upstream_tests evidence must run the original upstream workspace test suite",
        "upstream evidence must not use a filtered cargo test command",
    );
}

#[test]
fn doctor_project_support_rejects_upstream_tests_outside_replacement_root() {
    let project = TestProject::new("doctor-project-support-upstream-not-replacement");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let evidence_path = project.root.join(".kobo/evidence/upstream-tests.json");
    let mut evidence: Value =
        serde_json::from_str(&fs::read_to_string(&evidence_path).expect("evidence should read"))
            .expect("evidence should parse");
    evidence["replacement_root"] = serde_json::json!("target/generated-watchexec");
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&evidence).expect("evidence should serialize"),
    )
    .expect("evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "wrong replacement-root report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "original_upstream_tests evidence upstream test command must run from the Kobo-generated replacement root",
        "upstream tests should exercise the replacement root",
    );
}

#[test]
fn doctor_project_support_rejects_incomplete_reviewer_comparison() {
    let project = TestProject::new("doctor-project-support-incomplete-reviewer");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let reviewer_path = project.root.join(".kobo/evidence/reviewer-a.json");
    let mut reviewer_evidence: Value =
        serde_json::from_str(&fs::read_to_string(&reviewer_path).expect("reviewer should read"))
            .expect("reviewer should parse");
    let sections = reviewer_evidence["review_sections"]
        .as_array()
        .expect("review sections should exist")
        .iter()
        .filter(|section| section["name"].as_str() != Some("diagnostics"))
        .cloned()
        .collect::<Vec<_>>();
    reviewer_evidence["review_sections"] = serde_json::json!(sections);
    reviewer_evidence["review_sections"][0]["command_output_hash"] =
        serde_json::json!("not-a-command-hash");
    fs::write(
        &reviewer_path,
        serde_json::to_string_pretty(&reviewer_evidence).expect("reviewer should serialize"),
    )
    .expect("reviewer should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "incomplete-reviewer report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    let blockers = value["project_support"]["blockers"].to_string();
    assert_contains(
        &blockers,
        "reviewer-a reviewer report missing diagnostics comparison",
        "reviewer evidence must compare diagnostics independently",
    );
    assert_contains(
        &blockers,
        "reviewer-a reviewer behavior comparison is not tied to a command transcript",
        "reviewer comparisons must cite real command output",
    );
}

#[test]
fn doctor_project_support_rejects_missing_platform_observation() {
    let project = TestProject::new("doctor-project-support-missing-platform-observation");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let evidence_path = project.root.join(".kobo/evidence/filesystem-events.json");
    let mut evidence: Value =
        serde_json::from_str(&fs::read_to_string(&evidence_path).expect("evidence should read"))
            .expect("evidence should parse");
    let observations = evidence["platform_observations"]
        .as_array()
        .expect("platform observations should exist")
        .iter()
        .filter(|observation| observation["platform"].as_str() != Some("linux"))
        .cloned()
        .collect::<Vec<_>>();
    evidence["platform_observations"] = serde_json::json!(observations);
    fs::write(
        &evidence_path,
        serde_json::to_string_pretty(&evidence).expect("evidence should serialize"),
    )
    .expect("evidence should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "missing platform observation report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "filesystem_events evidence missing linux watcher platform observation",
        "platform evidence should cover required behavior per platform",
    );
}

#[test]
fn doctor_project_support_rejects_incomplete_future_release_evidence() {
    let project = TestProject::new("doctor-project-support-future-gates");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    let mut manifest: Value =
        serde_json::from_str(&manifest).expect("support manifest should parse");
    manifest["proof_debt_map_reports"] = Value::Null;
    manifest["independent_equivalence"]["mutation_tests"] = serde_json::json!(["watcher", "child"]);
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("support manifest should serialize"),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "future-gate report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "missing proof debt report agreement evidence",
        "project support gate should require proof/debt report agreement",
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "missing equivalence mutation test generated_backend_output",
        "project support gate should require generated backend mutation evidence",
    );
}

#[test]
fn doctor_project_support_rejects_incomplete_proof_debt_map() {
    let project = TestProject::new("doctor-project-support-incomplete-proof-map");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    let mut manifest: Value =
        serde_json::from_str(&manifest).expect("support manifest should parse");
    let entries = manifest["proof_debt_map"]
        .as_array()
        .expect("proof map should exist")
        .iter()
        .filter(|entry| entry["module"].as_str() != Some("crates/events/src/lib.rs"))
        .cloned()
        .collect::<Vec<_>>();
    manifest["proof_debt_map"] = serde_json::json!(entries);
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("support manifest should serialize"),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "incomplete-proof-map report should stay inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "proof debt map missing upstream module crates/events/src/lib.rs",
        "proof debt map should classify every upstream module",
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

#[test]
fn doctor_project_support_accepts_adapter_source_declared_by_upstream_metadata() {
    let project = TestProject::new("doctor-project-support-upstream-adapter-source");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""kind": "terminal_helpers", "name": "kobo-terminal-boundary", "crate_source": {"kind": "std", "name": "std::io"}, "version_range": "std""#,
            r#""kind": "terminal_helpers", "name": "watchexec-cli", "crate_source": {"kind": "cargo_dependency", "name": "watchexec-cli"}, "version_range": "workspace""#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "upstream cargo-backed adapter source should be accepted",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(
        value["project_support"]["status"], "ready",
        "adapter sources may be proved by the upstream workspace metadata"
    );
}

#[test]
fn doctor_project_support_accepts_upstream_release_artifacts() {
    let project = TestProject::new("doctor-project-support-upstream-release-artifact");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest.replace(
            r#""release_artifacts": [".kobo/evidence/release.zip"]"#,
            r#""release_artifacts": ["target/release/watchexec"]"#,
        ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(
        &output,
        "upstream release artifact should keep the report inspectable",
    );
    let value = parse_stdout_json(&output);
    assert_eq!(
        value["project_support"]["status"], "ready",
        "release parity artifacts are relative to the upstream inventory root"
    );
}

#[test]
fn doctor_project_support_rejects_self_inventory() {
    let project = TestProject::new("doctor-project-support-self-inventory");
    write_project_files(&project);
    write_complete_support_manifest(&project);
    let manifest_path = project.root.join(".kobo/project-support.json");
    let manifest = fs::read_to_string(&manifest_path).expect("support manifest should read");
    fs::write(
        &manifest_path,
        manifest
            .replace(r#""root": "upstream/watchexec""#, r#""root": ".""#)
            .replace(
                r#""project_name": "watchexec""#,
                r#""project_name": "support-case""#,
            ),
    )
    .expect("support manifest should write");

    let output = run_kobo(
        &[s("doctor"), s("--project-support"), s("--json")],
        &project.root,
    );
    assert_success(&output, "self-inventory report should stay inspectable");
    let value = parse_stdout_json(&output);
    assert_eq!(value["project_support"]["status"], "blocked");
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "upstream inventory project_name must be watchexec",
        "project support should reject evidence pointed at the current Kobo project",
    );
    assert_contains(
        &value["project_support"]["blockers"].to_string(),
        "upstream inventory root must not be the Kobo project root",
        "project support should require separate upstream source evidence",
    );
}
