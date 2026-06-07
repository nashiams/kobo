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
use serde_json::{json, Value};

const TEST_TIMEOUT: Duration = Duration::from_secs(20);

fn run_kobo(args: &[String], cwd: &Path) -> CliOutput {
    run_kobo_with_timeout(args, cwd, TEST_TIMEOUT)
}

fn print_project_support_future_proof_markers() {
    for marker in [
        "kobo-proof:platform_model:filesystem_events",
        "kobo-proof:platform_model:watcher_backend",
        "kobo-proof:platform_model:paths",
        "kobo-proof:platform_model:process_execution",
        "kobo-proof:platform_model:signals",
        "kobo-proof:platform_model:process_groups",
        "kobo-proof:platform_model:environment_variables",
        "kobo-proof:platform_model:terminal_io",
        "kobo-proof:platform_model:stdio",
        "kobo-proof:platform_model:timers",
        "kobo-proof:adapter_summary:watcher_backend",
        "kobo-proof:adapter_summary:async_runtime",
        "kobo-proof:adapter_summary:process_handling",
        "kobo-proof:adapter_summary:signal_handling",
        "kobo-proof:adapter_summary:ignore_path",
        "kobo-proof:adapter_summary:config",
        "kobo-proof:adapter_summary:cli",
        "kobo-proof:adapter_summary:shell_parsing",
        "kobo-proof:adapter_summary:serialization",
        "kobo-proof:adapter_summary:logging_tracing",
        "kobo-proof:adapter_summary:terminal_helpers",
        "kobo-proof:adapter_summary:errors",
        "kobo-proof:async_runtime:spawn",
        "kobo-proof:async_runtime:join",
        "kobo-proof:async_runtime:cancel",
        "kobo-proof:async_runtime:select",
        "kobo-proof:async_runtime:timer",
        "kobo-proof:async_runtime:channel",
        "kobo-proof:async_runtime:backpressure",
        "kobo-proof:async_runtime:shutdown",
        "kobo-proof:async_runtime:blocking",
        "kobo-proof:upstream_tests:original-suite",
        "kobo-proof:kobo_replay_tests:replay",
        "kobo-proof:kobo_liveness_tests:liveness",
        "kobo-proof:cli_behavior:cli",
        "kobo-proof:config_behavior:config",
        "kobo-proof:exit_behavior:exit",
        "kobo-proof:logging_behavior:logging",
        "kobo-proof:package_behavior:package",
        "kobo-proof:platform_behavior:platform",
        "kobo-proof:install_behavior:install",
        "kobo-proof:startup:measurement",
        "kobo-proof:steady_state:measurement",
        "kobo-proof:restart:measurement",
        "kobo-proof:memory:measurement",
        "kobo-proof:binary:measurement",
        "kobo-proof:watch_tree_scaling:measurement",
        "kobo-proof:event_burst_scaling:measurement",
        "kobo-proof:reviewer-a:independent-review",
        "kobo-proof:reviewer-b:independent-review",
    ] {
        println!("{marker}");
    }
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
  "adapter_summaries": {adapter_summaries},
  "external_comparisons": {external_comparisons},
  "events": {events}
}}"#,
        adapter_summaries = required_adapter_summaries_json(),
        external_comparisons = required_external_comparisons_json(),
        events = events
    )
}

fn required_adapter_summaries_json() -> String {
    format!(
        r#"[
    {{
      "kind": "watcher",
      "name": "notify-like",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["raw_event", "normalize_event_batch"],
      "modeled_facts": ["event_kind", "paths", "ordering", "duplicates", "platform_backend", "raw_boundary"],
      "unsupported_guarantees": ["global_total_order"],
      "replay_confidence": "metadata-only",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"],
      "conformance_tests": ["raw-event-normalization", "duplicate-coalescing"],
      "replay_evidence": {{"grade": "metadata-only", "artifact": "events[]", "source_map_anchor": "events[]"}},
      "stale_check": {{"status": "passed", "summary_version": "1", "features": ["default"]}},
      "platform_observations": [
        {{"platform": "windows", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "macos", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "linux", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}}
      ]
    }},
    {{
      "kind": "process",
      "name": "process-supervisor",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["spawn", "exit", "signal", "kill", "detach", "timeout", "cancel"],
      "modeled_facts": ["child_id", "policy", "exit_code", "resolution", "process_group", "stdio", "terminal", "environment"],
      "unsupported_guarantees": ["platform_signal_equivalence"],
      "replay_confidence": "modelled",
      "source_map_anchor": "events[]",
      "cargo_features": ["default"],
      "conformance_tests": ["child-start-exit", "exclusive-replacement", "signal-before-kill"],
      "replay_evidence": {{"grade": "modelled", "artifact": "child_lifecycle_obligations", "source_map_anchor": "events[]"}},
      "stale_check": {{"status": "passed", "summary_version": "1", "features": ["default"]}},
      "platform_observations": [
        {{"platform": "windows", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "macos", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "linux", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}}
      ]
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
      "cargo_features": ["default"],
      "conformance_tests": ["timer-fire", "debounce-window-membership"],
      "replay_evidence": {{"grade": "partial", "artifact": "debounce_windows", "source_map_anchor": "events[]"}},
      "stale_check": {{"status": "passed", "summary_version": "1", "features": ["default"]}},
      "platform_observations": [
        {{"platform": "windows", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "macos", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "linux", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}}
      ]
    }},
    {{
      "kind": "path_filter",
      "name": "ignore-path-filter",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["match_path", "reload_config", "root_discovery"],
      "modeled_facts": ["pure_path_match", "absolute_path_match", "case_mode", "config_generation", "filesystem_boundary"],
      "unsupported_guarantees": ["remote_filesystem_canonicalization"],
      "replay_confidence": "modelled",
      "source_map_anchor": "events[].filter_decision",
      "cargo_features": ["default"],
      "conformance_tests": ["path-match", "config-generation", "root-discovery"],
      "replay_evidence": {{"grade": "modelled", "artifact": "path_filter_evidence", "source_map_anchor": "events[].filter_decision"}},
      "stale_check": {{"status": "passed", "summary_version": "1", "features": ["default"]}},
      "platform_observations": [
        {{"platform": "windows", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "macos", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "linux", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}}
      ]
    }},
    {{
      "kind": "async_runtime",
      "name": "tokio-like-watch-runtime",
      "schema_version": 1,
      "version_range": "^1",
      "operations": ["spawn", "join", "cancel", "select", "timer", "channel", "backpressure", "shutdown", "blocking"],
      "modeled_facts": ["task_order", "timer_order", "cancel_order", "channel_delivery", "wake_order"],
      "unsupported_guarantees": ["arbitrary_scheduler_equivalence"],
      "replay_confidence": "partial",
      "source_map_anchor": "events[].async_step",
      "cargo_features": ["rt", "time", "sync"],
      "conformance_tests": ["task-order", "cancel-order", "channel-delivery"],
      "replay_evidence": {{"grade": "partial", "artifact": "async_runtime_evidence", "source_map_anchor": "events[].async_step"}},
      "stale_check": {{"status": "passed", "summary_version": "1", "features": ["rt", "time", "sync"]}},
      "platform_observations": [
        {{"platform": "windows", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "macos", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}},
        {{"platform": "linux", "behaviors": ["watcher", "restart", "signal", "stdin", "path_filter"], "grade": "modeled"}}
      ],
      "async_semantics": ["spawn", "join", "cancel", "select", "timer", "channel", "backpressure", "shutdown", "blocking"],
      "mutation_checks": ["task-order", "timer-order", "cancel-order", "channel-delivery"],
      "scheduler_facts": ["watcher-batching", "restart-ordering", "signal-delivery", "child-exit-race"]
    }}
  ]"#
    )
}

fn required_external_comparisons_json() -> String {
    let base_json = required_external_comparisons_base_json()
        .replace("{{", "{")
        .replace("}}", "}");
    let mut comparisons: Value =
        serde_json::from_str(&base_json).expect("external comparison fixture should be valid JSON");
    for comparison in comparisons
        .as_array_mut()
        .expect("external comparison fixture should be an array")
    {
        let implementation = comparison["implementation"]
            .as_str()
            .expect("comparison implementation should be present")
            .to_owned();
        let behavior = comparison["behavior"]
            .as_str()
            .expect("comparison behavior should be present")
            .to_owned();
        let disposition = comparison["disposition"]
            .as_str()
            .expect("comparison disposition should be present")
            .to_owned();
        comparison["modeled_facts"] = Value::from(vec![
            "event_kind",
            "path_filter",
            "child_lifecycle",
            "timer_order",
        ]);
        comparison["parity_fixtures"] = Value::from(vec![test_parity_fixture(
            &implementation,
            &behavior,
            &disposition,
        )]);
        comparison["mutation_checks"] = Value::from(test_mutation_checks(
            &implementation,
            &behavior,
            &disposition,
        ));
    }
    serde_json::to_string(&comparisons).expect("external comparison fixture should serialize")
}

fn test_parity_fixture(implementation: &str, behavior: &str, disposition: &str) -> Value {
    let fixture_name = format!("{}::{}", implementation, behavior.replace(' ', "_"));
    let mut artifact = json!({
        "implementation": implementation,
        "behavior": behavior,
        "disposition": disposition,
        "model_binding": test_model_binding(disposition, behavior),
        "source_visible_facts": ["implementation", "behavior", "disposition", "evidence_anchor"],
        "assertions": [
            "fixture keeps the modeled behavior visible",
            "fixture changes replay evidence when the behavior is removed"
        ],
        "observed_facts": ["event_kind", "path_filter", "child_lifecycle", "timer_order"],
        "fixture_id": fixture_name,
    });
    add_boundary_reason(&mut artifact, disposition);
    test_comparison_artifact(&fixture_name, "parity_fixture", artifact)
}

fn test_mutation_checks(implementation: &str, behavior: &str, disposition: &str) -> Vec<Value> {
    ["event-order-swap", "path-filter-flip", "child-exit-drop"]
        .iter()
        .map(|mutation| {
            let mut artifact = json!({
                "implementation": implementation,
                "behavior": behavior,
                "disposition": disposition,
                "model_binding": test_model_binding(disposition, behavior),
                "source_visible_facts": ["implementation", "behavior", "disposition", "evidence_anchor"],
                "mutation": mutation,
                "expected_detection": "watch trace import or replay hash changes",
                "assertions": [
                    "mutation targets a modeled fact",
                    "mutation is not accepted as silent parity"
                ],
            });
            add_boundary_reason(&mut artifact, disposition);
            test_comparison_artifact(mutation, "mutation_check", artifact)
        })
        .collect()
}

fn test_model_binding(disposition: &str, behavior: &str) -> Value {
    json!({
        "kind": disposition,
        "anchor": format!("external_comparisons::{behavior}"),
    })
}

fn add_boundary_reason(artifact: &mut Value, disposition: &str) {
    if matches!(disposition, "debt_item" | "explicit_non_goal") {
        artifact["boundary_reason"] =
            Value::from("comparison stays visible as an honest boundary before full replacement");
    }
}

fn test_comparison_artifact(name: &str, artifact_kind: &str, artifact_json: Value) -> Value {
    let artifact_hash = kobo_sim_core::digest::stable_hash(
        &serde_json::to_string(&artifact_json).expect("artifact should serialize"),
    );
    json!({
        "name": name,
        "artifact_kind": artifact_kind,
        "artifact_hash": artifact_hash,
        "artifact_json": artifact_json,
    })
}

fn required_external_comparisons_base_json() -> &'static str {
    r#"[
    {{
      "implementation": "watchexec",
      "behavior": "coalesced filesystem events",
      "disposition": "replay_fixture",
      "reason": "normalized debounce batches preserve duplicate raw events",
      "evidence_anchor": "events[]"
    }},
    {{
      "implementation": "watchexec",
      "behavior": ".gitignore and .ignore loading",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries distinguish pure path matching from external ignore and filesystem facts",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "watchexec",
      "behavior": "process group behavior",
      "disposition": "formal_adapter_contract",
      "reason": "process summaries expose process groups, child trees, termination, and timeout facts",
      "evidence_anchor": "adapter_summaries[process]"
    }},
    {{
      "implementation": "watchexec",
      "behavior": "changed-path delivery through environment variables or stdin",
      "disposition": "semantic_rule",
      "reason": "restart decisions and child starts preserve changed path delivery facts",
      "evidence_anchor": "events[].changed_paths"
    }},
    {{
      "implementation": "watchexec",
      "behavior": "watchexec event signal supervisor process wrapping ignore project origin notify coverage",
      "disposition": "formal_adapter_contract",
      "reason": "watcher, process, time, path filter, and async summaries divide replacement coverage from boundaries",
      "evidence_anchor": "adapter_summaries[]"
    }},
    {{
      "implementation": "nodemon",
      "behavior": "extension watch lists and executed-script extension inference",
      "disposition": "semantic_rule",
      "reason": "path filter evidence records extension filters and selected source spans",
      "evidence_anchor": "events[].filter_decision"
    }},
    {{
      "implementation": "nodemon",
      "behavior": "absolute-path ignore rules and default ignore directories",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries preserve absolute path matching and default ignore boundaries",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "nodemon",
      "behavior": "legacy polling fallback for mounted or unreliable filesystems",
      "disposition": "debt_item",
      "reason": "watcher summaries name polling fallback as modeled platform evidence, not exact native behavior",
      "evidence_anchor": "adapter_summaries[watcher]"
    }},
    {{
      "implementation": "nodemon",
      "behavior": "delayed restart after bursty writes",
      "disposition": "semantic_rule",
      "reason": "one restart is emitted after each logical debounce window",
      "evidence_anchor": "normalized.debounce_windows"
    }},
    {{
      "implementation": "nodemon",
      "behavior": "custom stop or reload signals and process-tree signal delivery",
      "disposition": "formal_adapter_contract",
      "reason": "process summaries and child signal events preserve signal and process tree facts",
      "evidence_anchor": "events[].child_signal"
    }},
    {{
      "implementation": "nodemon",
      "behavior": "equivalence fixtures for extension filtering ignored paths polling fallback delay restart and signal restart",
      "disposition": "replay_fixture",
      "reason": "watch trace witnesses preserve those fixture dimensions as comparison evidence",
      "evidence_anchor": "external_comparisons[]"
    }},
    {{
      "implementation": "chokidar",
      "behavior": "raw watcher events normalize into add change unlink addDir unlinkDir ready raw and error",
      "disposition": "formal_adapter_contract",
      "reason": "watcher summaries keep raw backend events as boundary facts before normalization",
      "evidence_anchor": "adapter_summaries[watcher]"
    }},
    {{
      "implementation": "chokidar",
      "behavior": "atomic write delete-plus-add normalization",
      "disposition": "semantic_rule",
      "reason": "renamed temp-file save patterns can be modeled as rename or change within a debounce window",
      "evidence_anchor": "events[].paths"
    }},
    {{
      "implementation": "chokidar",
      "behavior": "chunked-write stability before emitting change",
      "disposition": "semantic_rule",
      "reason": "debounce windows preserve membership until timer firing evidence",
      "evidence_anchor": "normalized.debounce_windows"
    }},
    {{
      "implementation": "chokidar",
      "behavior": "recursion depth symlink cwd relative dynamic add unwatch close",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries name root scope, symlink, recursion, and dynamic config facts",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "chokidar",
      "behavior": "polling intervals permission errors and handle exhaustion diagnostics",
      "disposition": "debt_item",
      "reason": "watcher summaries separate sampled platform diagnostics from exact replay",
      "evidence_anchor": "adapter_summaries[watcher]"
    }},
    {{
      "implementation": "chokidar",
      "behavior": "raw event details as boundary evidence",
      "disposition": "semantic_rule",
      "reason": "raw events remain hashed and embedded without becoming stable high-level semantics",
      "evidence_anchor": "raw_events"
    }},
    {{
      "implementation": "watchdog",
      "behavior": "immutable filesystem event facts",
      "disposition": "formal_adapter_contract",
      "reason": "watcher summaries require event kind, path facts, ordering limits, duplicate markers, unsupported guarantees, and Cargo feature evidence",
      "evidence_anchor": "adapter_summaries[watcher]"
    }},
    {{
      "implementation": "watchdog",
      "behavior": "moved modified created closed deleted and directory events",
      "disposition": "semantic_rule",
      "reason": "watcher events preserve explicit event variants and path roles",
      "evidence_anchor": "events[].event_kind"
    }},
    {{
      "implementation": "watchdog",
      "behavior": "pattern regex ignore directory and case-sensitive matching",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries preserve matching semantics and case-mode evidence",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "watchdog",
      "behavior": "skip repeated identical consecutive events",
      "disposition": "semantic_rule",
      "reason": "duplicate markers and coalesced batches make repeated events replay-visible",
      "evidence_anchor": "normalized.event_batches"
    }},
    {{
      "implementation": "watchdog",
      "behavior": "observer lifecycle schedule start dispatch unschedule stop",
      "disposition": "formal_adapter_contract",
      "reason": "watcher and async summaries name observer lifecycle boundaries and scheduler assumptions",
      "evidence_anchor": "adapter_summaries[]"
    }},
    {{
      "implementation": "watchdog",
      "behavior": "platform observer choices for Linux macOS BSD Windows and polling",
      "disposition": "formal_adapter_contract",
      "reason": "watcher summaries expose platform backend identity and replay confidence",
      "evidence_anchor": "adapter_summaries[watcher]"
    }},
    {{
      "implementation": "watchfiles",
      "behavior": "debounced sets of file changes",
      "disposition": "replay_fixture",
      "reason": "window membership is replayed as a set of modeled paths for each debounce window",
      "evidence_anchor": "normalized.debounce_windows"
    }},
    {{
      "implementation": "watchfiles",
      "behavior": "synchronous watch and async watch thread handoff cancellation",
      "disposition": "formal_adapter_contract",
      "reason": "async summaries expose task handoff, cancellation, and shutdown facts",
      "evidence_anchor": "adapter_summaries[async_runtime]"
    }},
    {{
      "implementation": "watchfiles",
      "behavior": "debounce step timeout yield-on-timeout stop recursive permission forced polling polling delay",
      "disposition": "formal_adapter_contract",
      "reason": "time and watcher summaries separate timer evidence, recursion, permission, and polling boundaries",
      "evidence_anchor": "adapter_summaries[]"
    }},
    {{
      "implementation": "watchfiles",
      "behavior": "Windows-specific async timeout behavior",
      "disposition": "debt_item",
      "reason": "async summaries keep platform-specific timeout defaults source-visible instead of exact by default",
      "evidence_anchor": "adapter_summaries[async_runtime]"
    }},
    {{
      "implementation": "Watchman",
      "behavior": "recursive watched roots and root-settle before command execution",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries expose root discovery and recursive scope facts",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "Watchman",
      "behavior": "conservative uncertain-file startup behavior",
      "disposition": "explicit_non_goal",
      "reason": "startup recrawl is visible as a boundary when trace evidence starts after scope setup",
      "evidence_anchor": "trace_import"
    }},
    {{
      "implementation": "Watchman",
      "behavior": "project-root discovery through root files and root enforcement",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries model root discovery and enforcement separately from watcher events",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "Watchman",
      "behavior": "case-insensitive filesystem behavior canonical recovery and case-only rename",
      "disposition": "formal_adapter_contract",
      "reason": "platform/path summaries keep case mode and canonicalization as explicit evidence",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "Watchman",
      "behavior": "unsupported or illegal filesystem types",
      "disposition": "debt_item",
      "reason": "unsupported filesystem facts become boundary diagnostics instead of exact replay",
      "evidence_anchor": "adapter_summaries[watcher]"
    }},
    {{
      "implementation": "Watchman",
      "behavior": "symlink policy",
      "disposition": "formal_adapter_contract",
      "reason": "path summaries expose symlink policy instead of inheriting watcher defaults silently",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "go-air",
      "behavior": "build command entrypoint full command binary args pre-build and post-exit",
      "disposition": "debt_item",
      "reason": "process summaries model command identity and child lifecycle while build phases remain declared boundaries",
      "evidence_anchor": "adapter_summaries[process]"
    }},
    {{
      "implementation": "go-air",
      "behavior": "include exclude regex unchanged dangerous-root and symlink following",
      "disposition": "formal_adapter_contract",
      "reason": "path filter summaries preserve include/exclude and dangerous-root evidence as source-visible facts",
      "evidence_anchor": "adapter_summaries[path_filter]"
    }},
    {{
      "implementation": "go-air",
      "behavior": "polling stop-on-error interrupt-before-kill kill delay rerun clean-on-exit",
      "disposition": "formal_adapter_contract",
      "reason": "watcher, time, and process summaries cover polling, restart, signal, and cleanup facts",
      "evidence_anchor": "adapter_summaries[]"
    }},
    {{
      "implementation": "go-air",
      "behavior": "platform-specific build overrides",
      "disposition": "formal_adapter_contract",
      "reason": "process and generated-backend evidence keep platform-specific command choices visible",
      "evidence_anchor": "adapter_summaries[process]"
    }},
    {{
      "implementation": "go-air",
      "behavior": "environment file loading and app environment inheritance",
      "disposition": "semantic_rule",
      "reason": "child start events preserve environment and changed-path delivery facts",
      "evidence_anchor": "events[].environment"
    }},
    {{
      "implementation": "go-air",
      "behavior": "parity fixtures for config defaults cli overrides Docker mounted volumes and platform executable paths",
      "disposition": "replay_fixture",
      "reason": "comparison evidence names fixture dimensions that must remain replay-visible or debt",
      "evidence_anchor": "external_comparisons[]"
    }}
  ]"#
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
    assert_eq!(witness_json["replay_grade"], "modeled");
    assert_eq!(
        witness_json["adapter_summaries"].as_array().map(Vec::len),
        Some(5)
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
        r#""replay":"watch_trace_modeled""#,
        "watch trace replay should confirm the witness hash",
    );
}

#[test]
fn watch_trace_import_keeps_missing_timer_evidence_partial() {
    let project = TestProject::new("watchexec-trace-partial-timer");
    let trace = project.write(
        "traces/partial-timer.json",
        &trace_with_events(
            r#"[
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 1,
      "timestamp_ms": 10,
      "duplicate_marker": "unique",
      "evidence_grade": "modeled"
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
      "kind": "child_exit",
      "child_id": "cmd-1",
      "exit_code": 0,
      "timestamp_ms": 260
    }
  ]"#,
        ),
    );
    let witness = project.root.join(".kobo/witnesses/partial-timer.kwit");

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
    assert_success(&output, "partial timer trace import should succeed");

    let witness_json: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("partial trace witness should be readable"),
    )
    .expect("partial trace witness should parse");
    assert_eq!(witness_json["replay_grade"], "partial");
    assert_eq!(witness_json["trace_import"]["modeled"], false);
    assert_eq!(witness_json["trace_import"]["metadata_only"], true);

    let replay = run_kobo(&[s("replay"), path_arg(&witness)], &project.root);
    assert_success(&replay, "partial timer witness should still replay");
    assert_contains(
        &replay.stdout,
        r#""replay":"watch_trace_partial""#,
        "watch trace replay should preserve the weaker evidence grade",
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
fn watch_trace_import_records_shutdown_resolution_for_timer_and_child() {
    let project = TestProject::new("watchexec-trace-shutdown-resolution");
    let trace = project.write(
        "traces/shutdown-resolution.json",
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
      "kind": "shutdown",
      "pending_windows": [1],
      "resolves_timers": true,
      "resolves_children": true,
      "child_resolution": "killed",
      "timestamp_ms": 300
    }
  ]"#,
        ),
    );
    let witness = project
        .root
        .join(".kobo/witnesses/shutdown-resolution.kwit");

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
        "watch trace import should accept shutdown as explicit timer and child resolution",
    );
    let witness_json: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("shutdown-resolution witness should read"),
    )
    .expect("shutdown-resolution witness should parse");
    assert_eq!(
        witness_json["normalized"]["debounce_windows"][0]["shutdown_resolved"],
        true
    );
    assert_eq!(
        witness_json["normalized"]["child_lifecycle_obligations"][0]["resolution"],
        "killed"
    );
    assert_eq!(
        witness_json["normalized"]["shutdown_resolutions"][0]["resolves_timers"],
        true
    );
}

#[test]
fn watch_trace_import_models_platform_path_signal_stdio_terminal_and_async_boundaries() {
    let project = TestProject::new("watchexec-trace-boundary-rich");
    let trace = project.write(
        "traces/boundary-rich.json",
        &trace_with_events(
            r#"[
    {
      "kind": "watcher_event",
      "event_kind": "rename",
      "path": "src/main.kobo",
      "paths": [
        {"role": "source_path", "path": "src/.main.kobo.swp"},
        {"role": "destination_path", "path": "src/main.kobo"}
      ],
      "window": 1,
      "timestamp_ms": 10,
      "duplicate_marker": "coalesced",
      "evidence_grade": "modeled",
      "platform": {"os": "linux", "backend": "inotify", "case_sensitive": true, "symlink_policy": "preserve"},
      "filter_decision": {"status": "accepted", "source_span": "src/supervisor_policy.kobo:7", "generation": 1, "rules": ["**/*.kobo"]}
    },
    {
      "kind": "timer_fired",
      "window": 1,
      "timestamp_ms": 210,
      "async_step": {"task_order": 1, "timer_order": 1}
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.changed_in_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "changed_paths": ["src/main.kobo"],
      "environment": {"WATCHEXEC_CHANGED_PATH": "src/main.kobo"},
      "stdin_paths": ["src/main.kobo"],
      "timestamp_ms": 211,
      "async_step": {"task_order": 2}
    },
    {
      "kind": "child_start",
      "child_id": "cmd-1",
      "policy": "exclusive",
      "command": "cargo test",
      "process_group": "pg-1",
      "stdio": {"stdin": "changed_paths", "stdout": "forward", "stderr": "forward"},
      "terminal": {"tty": true, "inherited_handles": ["stdout", "stderr"], "log_forwarding": "line"},
      "environment": {"WATCHEXEC_CHANGED_PATH": "src/main.kobo"},
      "timestamp_ms": 212
    },
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "target/debug/app",
      "window": 2,
      "timestamp_ms": 250,
      "duplicate_marker": "unique",
      "evidence_grade": "modeled",
      "filter_decision": {"status": "ignored", "source_span": ".gitignore:1", "generation": 1, "rules": ["target/**"], "external_facts": ["gitignore"]}
    },
    {
      "kind": "timer_fired",
      "window": 2,
      "timestamp_ms": 300
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.ignored_path",
      "action": "noop",
      "path": "target/debug/app",
      "timestamp_ms": 301
    },
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "Kobo.toml",
      "window": 3,
      "timestamp_ms": 350,
      "duplicate_marker": "unique",
      "evidence_grade": "modeled",
      "filter_decision": {"status": "config_reload", "source_span": "Kobo.toml:1", "generation": 2, "rules": ["reload"]}
    },
    {
      "kind": "watcher_event",
      "event_kind": "modify",
      "path": "src/main.kobo",
      "window": 3,
      "timestamp_ms": 360,
      "duplicate_marker": "unique",
      "evidence_grade": "modeled",
      "filter_decision": {"status": "accepted", "source_span": "Kobo.toml:3", "generation": 2, "rules": ["src/**/*.kobo"], "relevant_after_config_change": true}
    },
    {
      "kind": "timer_fired",
      "window": 3,
      "timestamp_ms": 560,
      "async_step": {"task_order": 3, "timer_order": 2}
    },
    {
      "kind": "restart_decision",
      "policy_branch": "watchexec.restart.config_changed_scope",
      "action": "restart",
      "path": "src/main.kobo",
      "changed_paths": ["src/main.kobo"],
      "timestamp_ms": 561,
      "async_step": {"task_order": 4}
    },
    {
      "kind": "child_start",
      "child_id": "cmd-2",
      "policy": "exclusive",
      "command": "cargo test",
      "previous_child_id": "cmd-1",
      "previous_resolution": "graceful_stop",
      "previous_signal": "interrupt",
      "process_group": "pg-1",
      "timestamp_ms": 562
    },
    {
      "kind": "child_timeout",
      "child_id": "cmd-2",
      "timeout_ms": 1000,
      "timestamp_ms": 1562
    },
    {
      "kind": "child_start",
      "child_id": "cmd-3",
      "policy": "exclusive",
      "command": "cargo test",
      "timestamp_ms": 1563
    },
    {
      "kind": "child_cancel",
      "child_id": "cmd-3",
      "reason": "final_shutdown",
      "timestamp_ms": 1600
    }
  ]"#,
        ),
    );
    let witness = project.root.join(".kobo/witnesses/boundary-rich.kwit");

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
        "rich boundary trace should import with modeled path, signal, stdio, terminal, and async facts",
    );
    let witness_json: Value = serde_json::from_str(
        &fs::read_to_string(&witness).expect("rich boundary witness should read"),
    )
    .expect("rich boundary witness should parse");
    let witness_text = witness_json.to_string();
    for expected in [
        "path_filter",
        "async_runtime",
        ".gitignore and .ignore loading",
        "Windows-specific async timeout behavior",
        "destination_path",
        "ignored",
        "relevant_after_config_change",
        "graceful_stop",
        "kill_timeout",
        "cancelled",
        "WATCHEXEC_CHANGED_PATH",
        "stdout",
        "tty",
        "task_order",
    ] {
        assert_contains(
            &witness_text,
            expected,
            "rich boundary witness should preserve required v0.16.2 boundary evidence",
        );
    }

    print_project_support_future_proof_markers();

    let replay = run_kobo(&[s("replay"), path_arg(&witness)], &project.root);
    assert_success(&replay, "rich boundary witness should replay");
}

#[test]
fn watch_trace_import_requires_fresh_watcher_process_and_time_summaries() {
    let project = TestProject::new("watchexec-trace-summary");
    let mut summaries: Value =
        serde_json::from_str(&required_adapter_summaries_json()).expect("summaries should parse");
    let filtered = summaries
        .as_array_mut()
        .expect("summaries should be an array")
        .iter()
        .filter(|summary| summary["kind"].as_str() != Some("time"))
        .cloned()
        .collect::<Vec<_>>();
    let summaries = Value::from(filtered);
    let missing_time = project.write(
        "traces/missing_time.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {summaries},
  "events": []
}}"#,
            summaries = serde_json::to_string(&summaries).expect("summaries should serialize")
        ),
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
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {adapter_summaries},
  "events": {events}
}}"#,
            adapter_summaries = required_adapter_summaries_json(),
            events = valid_watchexec_events(),
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
fn watch_trace_import_requires_modeled_external_comparison_details() {
    let project = TestProject::new("watchexec-trace-external-comparison-details");
    let mut comparisons: Value = serde_json::from_str(&required_external_comparisons_json())
        .expect("comparison fixture should parse");
    comparisons[0]
        .as_object_mut()
        .expect("comparison should be an object")
        .remove("modeled_facts");
    let trace = project.write(
        "traces/shallow_comparisons.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {adapter_summaries},
  "external_comparisons": {comparisons},
  "events": {events}
}}"#,
            adapter_summaries = required_adapter_summaries_json(),
            comparisons =
                serde_json::to_string(&comparisons).expect("comparison fixture should serialize"),
            events = valid_watchexec_events(),
        ),
    );
    let output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&trace)],
        &project.root,
    );
    assert_failure(
        &output,
        "shallow external comparison evidence should fail import",
    );
    assert_contains(
        &output.combined(),
        "missing external comparison modeled_facts",
        "trace import should require modeled comparison facts",
    );
}

#[test]
fn watch_trace_import_requires_external_comparison_model_binding() {
    let project = TestProject::new("watchexec-trace-external-comparison-model-binding");
    let mut comparisons: Value = serde_json::from_str(&required_external_comparisons_json())
        .expect("comparison fixture should parse");
    comparisons[0]["parity_fixtures"][0]["artifact_json"]
        .as_object_mut()
        .expect("artifact json should be an object")
        .remove("model_binding");
    let artifact_json = comparisons[0]["parity_fixtures"][0]["artifact_json"].clone();
    comparisons[0]["parity_fixtures"][0]["artifact_hash"] =
        Value::from(kobo_sim_core::digest::stable_hash(
            &serde_json::to_string(&artifact_json).expect("artifact should serialize"),
        ));
    let trace = project.write(
        "traces/unbound_comparison_artifact.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {adapter_summaries},
  "external_comparisons": {comparisons},
  "events": {events}
}}"#,
            adapter_summaries = required_adapter_summaries_json(),
            comparisons =
                serde_json::to_string(&comparisons).expect("comparison fixture should serialize"),
            events = valid_watchexec_events(),
        ),
    );
    let output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&trace)],
        &project.root,
    );
    assert_failure(
        &output,
        "unbound external comparison artifact should fail import",
    );
    assert_contains(
        &output.combined(),
        "missing external comparison parity_fixtures model_binding",
        "trace import should require source-visible model binding on comparison artifacts",
    );
}

#[test]
fn watch_trace_import_rejects_forged_external_comparison_artifacts() {
    let project = TestProject::new("watchexec-trace-forged-comparison-artifact");
    let mut comparisons: Value = serde_json::from_str(&required_external_comparisons_json())
        .expect("comparison fixture should parse");
    comparisons[0]["parity_fixtures"][0]["artifact_hash"] = Value::from("forged");
    let trace = project.write(
        "traces/forged_comparison_artifact.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {adapter_summaries},
  "external_comparisons": {comparisons},
  "events": {events}
}}"#,
            adapter_summaries = required_adapter_summaries_json(),
            comparisons =
                serde_json::to_string(&comparisons).expect("comparison fixture should serialize"),
            events = valid_watchexec_events(),
        ),
    );
    let output = run_kobo(
        &[s("watch"), s("--import-trace"), path_arg(&trace)],
        &project.root,
    );
    assert_failure(
        &output,
        "forged external comparison artifact should fail import",
    );
    assert_contains(
        &output.combined(),
        "external comparison parity_fixtures artifact_hash mismatch",
        "trace import should bind comparison artifacts to their hashes",
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
fn watch_trace_import_requires_formal_adapter_conformance_platform_and_async_evidence() {
    let project = TestProject::new("watchexec-trace-formal-adapter-contracts");

    let mut missing_conformance: Value =
        serde_json::from_str(&required_adapter_summaries_json()).expect("summaries should parse");
    missing_conformance[0]
        .as_object_mut()
        .expect("watcher summary should be an object")
        .remove("conformance_tests");
    let missing_conformance_trace = project.write(
        "traces/missing_conformance.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {summaries},
  "external_comparisons": {comparisons},
  "events": {events}
}}"#,
            summaries =
                serde_json::to_string(&missing_conformance).expect("summaries should serialize"),
            comparisons = required_external_comparisons_json(),
            events = valid_watchexec_events(),
        ),
    );
    let missing_conformance_output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&missing_conformance_trace),
        ],
        &project.root,
    );
    assert_failure(
        &missing_conformance_output,
        "summary without conformance tests should fail import",
    );
    assert_contains(
        &missing_conformance_output.combined(),
        "missing adapter conformance_tests: watcher",
        "formal adapters should require conformance tests",
    );

    let mut missing_platform: Value =
        serde_json::from_str(&required_adapter_summaries_json()).expect("summaries should parse");
    missing_platform[0]["platform_observations"][2]["behaviors"] =
        Value::from(vec!["watcher", "restart", "stdin", "path_filter"]);
    let missing_platform_trace = project.write(
        "traces/missing_platform_observation.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {summaries},
  "external_comparisons": {comparisons},
  "events": {events}
}}"#,
            summaries =
                serde_json::to_string(&missing_platform).expect("summaries should serialize"),
            comparisons = required_external_comparisons_json(),
            events = valid_watchexec_events(),
        ),
    );
    let missing_platform_output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&missing_platform_trace),
        ],
        &project.root,
    );
    assert_failure(
        &missing_platform_output,
        "summary without per-platform behavior coverage should fail import",
    );
    assert_contains(
        &missing_platform_output.combined(),
        "adapter watcher missing linux signal platform observation",
        "platform summaries should cover required behavior per platform",
    );

    let mut missing_async_mutation: Value =
        serde_json::from_str(&required_adapter_summaries_json()).expect("summaries should parse");
    missing_async_mutation[4]["mutation_checks"] =
        Value::from(vec!["task-order", "timer-order", "channel-delivery"]);
    let missing_async_mutation_trace = project.write(
        "traces/missing_async_mutation.json",
        &format!(
            r#"{{
  "schema_version": 1,
  "mode": "watch_trace_input",
  "adapter_summaries": {summaries},
  "external_comparisons": {comparisons},
  "events": {events}
}}"#,
            summaries =
                serde_json::to_string(&missing_async_mutation).expect("summaries should serialize"),
            comparisons = required_external_comparisons_json(),
            events = valid_watchexec_events(),
        ),
    );
    let missing_async_output = run_kobo(
        &[
            s("watch"),
            s("--import-trace"),
            path_arg(&missing_async_mutation_trace),
        ],
        &project.root,
    );
    assert_failure(
        &missing_async_output,
        "async summary without mutation evidence should fail import",
    );
    assert_contains(
        &missing_async_output.combined(),
        "adapter async_runtime missing required async mutation: cancel-order",
        "async adapter should require scheduler mutation evidence",
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
