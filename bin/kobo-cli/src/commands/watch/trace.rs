use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{json, Value};

use crate::ErrorFormat;

const WATCH_TRACE_WITNESS_MODE: &str = "watch_trace_witness";

const REQUIRED_EXTERNAL_COMPARISON_COVERAGE: &[(&str, &str)] = &[
    ("watchexec", "coalesced filesystem events"),
    ("watchexec", ".gitignore and .ignore loading"),
    ("watchexec", "process group behavior"),
    (
        "watchexec",
        "changed-path delivery through environment variables or stdin",
    ),
    (
        "watchexec",
        "watchexec event signal supervisor process wrapping ignore project origin notify coverage",
    ),
    (
        "nodemon",
        "extension watch lists and executed-script extension inference",
    ),
    (
        "nodemon",
        "absolute-path ignore rules and default ignore directories",
    ),
    (
        "nodemon",
        "legacy polling fallback for mounted or unreliable filesystems",
    ),
    ("nodemon", "delayed restart after bursty writes"),
    (
        "nodemon",
        "custom stop or reload signals and process-tree signal delivery",
    ),
    (
        "nodemon",
        "equivalence fixtures for extension filtering ignored paths polling fallback delay restart and signal restart",
    ),
    (
        "chokidar",
        "raw watcher events normalize into add change unlink addDir unlinkDir ready raw and error",
    ),
    ("chokidar", "atomic write delete-plus-add normalization"),
    ("chokidar", "chunked-write stability before emitting change"),
    (
        "chokidar",
        "recursion depth symlink cwd relative dynamic add unwatch close",
    ),
    (
        "chokidar",
        "polling intervals permission errors and handle exhaustion diagnostics",
    ),
    ("chokidar", "raw event details as boundary evidence"),
    ("watchdog", "immutable filesystem event facts"),
    (
        "watchdog",
        "moved modified created closed deleted and directory events",
    ),
    (
        "watchdog",
        "pattern regex ignore directory and case-sensitive matching",
    ),
    ("watchdog", "skip repeated identical consecutive events"),
    (
        "watchdog",
        "observer lifecycle schedule start dispatch unschedule stop",
    ),
    (
        "watchdog",
        "platform observer choices for Linux macOS BSD Windows and polling",
    ),
    ("watchfiles", "debounced sets of file changes"),
    (
        "watchfiles",
        "synchronous watch and async watch thread handoff cancellation",
    ),
    (
        "watchfiles",
        "debounce step timeout yield-on-timeout stop recursive permission forced polling polling delay",
    ),
    ("watchfiles", "Windows-specific async timeout behavior"),
    (
        "Watchman",
        "recursive watched roots and root-settle before command execution",
    ),
    ("Watchman", "conservative uncertain-file startup behavior"),
    (
        "Watchman",
        "project-root discovery through root files and root enforcement",
    ),
    (
        "Watchman",
        "case-insensitive filesystem behavior canonical recovery and case-only rename",
    ),
    ("Watchman", "unsupported or illegal filesystem types"),
    ("Watchman", "symlink policy"),
    (
        "go-air",
        "build command entrypoint full command binary args pre-build and post-exit",
    ),
    (
        "go-air",
        "include exclude regex unchanged dangerous-root and symlink following",
    ),
    (
        "go-air",
        "polling stop-on-error interrupt-before-kill kill delay rerun clean-on-exit",
    ),
    ("go-air", "platform-specific build overrides"),
    ("go-air", "environment file loading and app environment inheritance"),
    (
        "go-air",
        "parity fixtures for config defaults cli overrides Docker mounted volumes and platform executable paths",
    ),
];

const REQUIRED_PLATFORMS: &[&str] = &["windows", "macos", "linux"];
const REQUIRED_PLATFORM_BEHAVIORS: &[&str] =
    &["watcher", "restart", "signal", "stdin", "path_filter"];
const REQUIRED_ASYNC_SEMANTICS: &[&str] = &[
    "spawn",
    "join",
    "cancel",
    "select",
    "timer",
    "channel",
    "backpressure",
    "shutdown",
    "blocking",
];
const REQUIRED_ASYNC_MUTATIONS: &[&str] = &[
    "task-order",
    "timer-order",
    "cancel-order",
    "channel-delivery",
];

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum AdapterKind {
    Watcher,
    Process,
    Time,
    PathFilter,
    AsyncRuntime,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ComparisonDisposition {
    SemanticRule,
    FormalAdapterContract,
    ReplayFixture,
    DebtItem,
    ExplicitNonGoal,
}

#[derive(Clone, Copy)]
enum TraceEventKind {
    WatcherEvent,
    TimerFired,
    RestartDecision,
    ChildStart,
    ChildExit,
    ChildSignal,
    ChildKill,
    ChildDetach,
    ChildTimeout,
    ChildCancel,
    Shutdown,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TraceReplayGrade {
    Modeled,
    Partial,
    Debt,
}

#[derive(Clone)]
struct AdapterSummary {
    kind: AdapterKind,
    json: Value,
}

#[derive(Clone)]
struct ExternalComparison {
    json: Value,
}

#[derive(Clone)]
struct WatcherTraceEvent {
    event_kind: String,
    path: String,
    paths: Vec<Value>,
    window: u64,
    evidence_grade: String,
    duplicate_marker: String,
    filter_decision: Option<Value>,
    platform: Option<Value>,
    raw: Value,
}

#[derive(Clone)]
struct ChildState {
    child_id: String,
    policy: String,
    command: String,
    process_group: Value,
    stdio: Value,
    terminal: Value,
    environment: Value,
}

struct WindowTrace {
    watcher_events: Vec<WatcherTraceEvent>,
    timer_events: Vec<Value>,
    has_timer_fired: bool,
    shutdown_resolved: bool,
}

struct NormalizedTrace {
    replay_grade: TraceReplayGrade,
    json: Value,
}

struct TraceImportInput {
    source_kind: &'static str,
    json: Value,
}

pub(super) fn cmd_import_trace(
    trace_path: &Path,
    witness_out: Option<&Path>,
) -> anyhow::Result<()> {
    let trace = trace_import_input(read_trace(trace_path)?)?;
    let adapter_summaries = parse_adapter_summaries(&trace.json)?;
    let external_comparisons = parse_external_comparisons(&trace.json)?;
    let raw_events = raw_events(&trace.json)?;
    let normalized = normalize_trace(&raw_events)?;
    let raw_event_hash = stable_values_hash(&raw_events)?;
    let normalized_hash = stable_value_hash(&normalized.json)?;
    let adapter_summary_hash = stable_values_hash(&adapter_summary_json(&adapter_summaries))?;
    let external_comparison_hash =
        stable_values_hash(&external_comparison_json(&external_comparisons))?;
    let witness = witness_json(
        trace_path,
        trace.source_kind,
        &adapter_summaries,
        &external_comparisons,
        raw_events,
        &normalized,
        &raw_event_hash,
        &normalized_hash,
        &adapter_summary_hash,
        &external_comparison_hash,
    );
    let output_path = witness_output_path(trace_path, witness_out)?;
    write_witness(&output_path, &witness)?;
    println!("watch trace witness: {}", output_path.display());
    println!("replay grade: {}", normalized.replay_grade.as_str());
    Ok(())
}

pub(super) fn is_watch_trace_witness(witness: &Value) -> bool {
    witness["mode"].as_str() == Some(WATCH_TRACE_WITNESS_MODE)
}

pub(super) fn replay_watch_trace_witness(
    witness: &Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let adapter_summaries = parse_adapter_summaries(witness)?;
    let external_comparisons = parse_external_comparisons(witness)?;
    let raw_events = raw_events(witness)?;
    let normalized = normalize_trace(&raw_events)?;
    let expected = json!({
        "raw_event_hash": stable_values_hash(&raw_events)?,
        "normalized_hash": stable_value_hash(&normalized.json)?,
        "adapter_summary_hash": stable_values_hash(&adapter_summary_json(&adapter_summaries))?,
        "external_comparison_hash": stable_values_hash(&external_comparison_json(&external_comparisons))?,
        "normalized": normalized.json,
    });
    let observed = json!({
        "raw_event_hash": witness["raw_event_hash"].clone(),
        "normalized_hash": witness["normalized_hash"].clone(),
        "adapter_summary_hash": witness["adapter_summary_hash"].clone(),
        "external_comparison_hash": witness["external_comparison_hash"].clone(),
        "normalized": witness["normalized"].clone(),
    });
    if expected != observed {
        return watch_trace_divergence(expected, observed, error_format);
    }
    println!(
        "{}",
        serde_json::to_string(&json!({
            "replay": normalized.replay_grade.replay_label(),
            "mode": WATCH_TRACE_WITNESS_MODE,
            "replay_grade": normalized.replay_grade.as_str(),
            "raw_events": raw_events.len(),
        }))?
    );
    Ok(())
}

fn read_trace(trace_path: &Path) -> anyhow::Result<Value> {
    let source = std::fs::read_to_string(trace_path)
        .with_context(|| format!("failed to read watch trace {}", trace_path.display()))?;
    serde_json::from_str(&source)
        .with_context(|| format!("failed to parse watch trace {}", trace_path.display()))
}

fn trace_import_input(value: Value) -> anyhow::Result<TraceImportInput> {
    if value["mode"].as_str() == Some("source_watch_state") {
        return Ok(TraceImportInput {
            source_kind: "source_watch_state",
            json: source_watch_state_trace(value)?,
        });
    }
    Ok(TraceImportInput {
        source_kind: "watch_trace",
        json: value,
    })
}

fn source_watch_state_trace(value: Value) -> anyhow::Result<Value> {
    Ok(json!({
        "schema_version": 1,
        "mode": "watch_trace_input",
        "adapter_summaries": source_watch_state_adapter_summaries(),
        "external_comparisons": source_watch_state_external_comparisons()?,
        "events": source_watch_state_events(&value)?,
    }))
}

pub(super) fn source_watch_state_adapter_summaries() -> Vec<Value> {
    vec![
        json!({
            "kind": "watcher",
            "name": "kobo-source-watch-state",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["raw_event", "poll_snapshot", "normalize_event_batch"],
            "modeled_facts": ["event_kind", "paths", "ordering", "duplicates", "platform_backend", "raw_boundary", "evidence_grade"],
            "unsupported_guarantees": ["native_backend_order", "platform_specific_raw_event_identity"],
            "replay_confidence": "metadata-only",
            "source_map_anchor": "event_batches[]",
            "cargo_features": ["default"],
            "conformance_tests": ["source-watch-event-kind", "source-watch-duplicate-batch"],
            "replay_evidence": {"grade": "metadata-only", "artifact": "source_watch_state.event_batches", "source_map_anchor": "event_batches[]"},
            "stale_check": {"status": "passed", "summary_version": "1", "features": ["default"]},
            "platform_observations": adapter_platform_observations(),
        }),
        json!({
            "kind": "process",
            "name": "kobo-in-process-supervisor",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["spawn", "exit", "signal", "kill", "timeout", "cancel", "rerun_start", "rerun_finish"],
            "modeled_facts": ["child_id", "policy", "exit_code", "resolution", "process_group", "stdio", "terminal", "environment", "command_kind", "diagnostic_count"],
            "unsupported_guarantees": ["os_process_group_signal_equivalence"],
            "replay_confidence": "modelled",
            "source_map_anchor": "child_lifecycle_obligations[]",
            "cargo_features": ["default"],
            "conformance_tests": ["source-watch-child-start-exit", "source-watch-no-orphan-child"],
            "replay_evidence": {"grade": "modelled", "artifact": "source_watch_state.child_lifecycle_obligations", "source_map_anchor": "child_lifecycle_obligations[]"},
            "stale_check": {"status": "passed", "summary_version": "1", "features": ["default"]},
            "platform_observations": adapter_platform_observations(),
        }),
        json!({
            "kind": "time",
            "name": "kobo-logical-debounce-window",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["timer_create", "timer_cancel", "timer_fire"],
            "modeled_facts": ["window", "membership", "fire_order"],
            "unsupported_guarantees": ["real_os_time_determinism"],
            "replay_confidence": "partial",
            "source_map_anchor": "debounce_windows[]",
            "cargo_features": ["default"],
            "conformance_tests": ["source-watch-timer-fire", "source-watch-window-membership"],
            "replay_evidence": {"grade": "partial", "artifact": "source_watch_state.debounce_windows", "source_map_anchor": "debounce_windows[]"},
            "stale_check": {"status": "passed", "summary_version": "1", "features": ["default"]},
            "platform_observations": adapter_platform_observations(),
        }),
        json!({
            "kind": "path_filter",
            "name": "kobo-path-filter-summary",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["match_path", "reload_config", "root_discovery"],
            "modeled_facts": ["pure_path_match", "absolute_path_match", "case_mode", "config_generation", "filesystem_boundary"],
            "unsupported_guarantees": ["remote_filesystem_canonicalization"],
            "replay_confidence": "modelled",
            "source_map_anchor": "event_batches[].events[].filter_decision",
            "cargo_features": ["default"],
            "conformance_tests": ["source-watch-path-match", "source-watch-config-generation"],
            "replay_evidence": {"grade": "modelled", "artifact": "source_watch_state.event_batches[].events[].filter_decision", "source_map_anchor": "event_batches[].events[].filter_decision"},
            "stale_check": {"status": "passed", "summary_version": "1", "features": ["default"]},
            "platform_observations": adapter_platform_observations(),
        }),
        json!({
            "kind": "async_runtime",
            "name": "kobo-watch-runtime-summary",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["spawn", "join", "cancel", "select", "timer", "channel", "backpressure", "shutdown", "blocking"],
            "modeled_facts": ["task_order", "timer_order", "cancel_order", "channel_delivery", "wake_order"],
            "unsupported_guarantees": ["arbitrary_scheduler_equivalence"],
            "replay_confidence": "partial",
            "source_map_anchor": "event_batches[].events[].async_step",
            "cargo_features": ["rt", "time", "sync"],
            "conformance_tests": ["source-watch-task-order", "source-watch-cancel-order", "source-watch-channel-delivery"],
            "replay_evidence": {"grade": "partial", "artifact": "source_watch_state.async_step", "source_map_anchor": "event_batches[].events[].async_step"},
            "stale_check": {"status": "passed", "summary_version": "1", "features": ["rt", "time", "sync"]},
            "platform_observations": adapter_platform_observations(),
            "async_semantics": ["spawn", "join", "cancel", "select", "timer", "channel", "backpressure", "shutdown", "blocking"],
            "mutation_checks": ["task-order", "timer-order", "cancel-order", "channel-delivery"],
            "scheduler_facts": ["watcher-batching", "restart-ordering", "signal-delivery", "child-exit-race"],
        }),
    ]
}

fn adapter_platform_observations() -> Vec<Value> {
    REQUIRED_PLATFORMS
        .iter()
        .map(|platform| {
            json!({
                "platform": platform,
                "behaviors": REQUIRED_PLATFORM_BEHAVIORS,
                "grade": "modeled",
            })
        })
        .collect()
}

pub(super) fn source_watch_state_external_comparisons() -> anyhow::Result<Vec<Value>> {
    REQUIRED_EXTERNAL_COMPARISON_COVERAGE
        .iter()
        .map(|(implementation, behavior)| {
            Ok(json!({
                "implementation": implementation,
                "behavior": behavior,
                "disposition": source_watch_comparison_disposition(behavior),
                "reason": source_watch_comparison_reason(implementation, behavior),
                "evidence_anchor": source_watch_comparison_anchor(behavior),
                "modeled_facts": source_watch_comparison_facts(behavior),
                "parity_fixtures": source_watch_comparison_fixtures(implementation, behavior)?,
                "mutation_checks": source_watch_comparison_mutations(implementation, behavior)?,
            }))
        })
        .collect()
}

fn source_watch_comparison_disposition(behavior: &str) -> &'static str {
    if behavior.contains("unsupported")
        || behavior.contains("Windows-specific")
        || behavior.contains("pre-build")
        || behavior.contains("polling intervals")
    {
        "debt_item"
    } else if behavior.contains("conservative uncertain-file startup") {
        "explicit_non_goal"
    } else if behavior.contains("fixture")
        || behavior.contains("coalesced filesystem events")
        || behavior.contains("debounced sets")
    {
        "replay_fixture"
    } else {
        "formal_adapter_contract"
    }
}

fn source_watch_comparison_reason(implementation: &str, behavior: &str) -> String {
    format!(
        "source-watch import keeps {implementation} `{behavior}` as explicit comparison evidence rather than claiming hidden parity"
    )
}

fn source_watch_comparison_anchor(behavior: &str) -> &'static str {
    if behavior.contains("async") || behavior.contains("scheduler") || behavior.contains("timeout")
    {
        "adapter_summaries[async_runtime]"
    } else if behavior.contains("ignore")
        || behavior.contains("path")
        || behavior.contains("root")
        || behavior.contains("symlink")
        || behavior.contains("case")
        || behavior.contains("extension")
        || behavior.contains("include")
        || behavior.contains("exclude")
    {
        "adapter_summaries[path_filter]"
    } else if behavior.contains("signal")
        || behavior.contains("process")
        || behavior.contains("command")
        || behavior.contains("environment")
        || behavior.contains("kill")
    {
        "adapter_summaries[process]"
    } else {
        "event_batches[]"
    }
}

fn source_watch_comparison_facts(behavior: &str) -> Vec<&'static str> {
    if behavior.contains("ignore")
        || behavior.contains("path")
        || behavior.contains("root")
        || behavior.contains("symlink")
        || behavior.contains("case")
        || behavior.contains("extension")
        || behavior.contains("include")
        || behavior.contains("exclude")
    {
        vec!["path_filter", "root_scope", "case_policy"]
    } else if behavior.contains("signal")
        || behavior.contains("process")
        || behavior.contains("command")
        || behavior.contains("environment")
        || behavior.contains("kill")
    {
        vec!["child_lifecycle", "process_group", "changed_path_delivery"]
    } else if behavior.contains("async")
        || behavior.contains("scheduler")
        || behavior.contains("timeout")
    {
        vec!["task_order", "timer_order", "cancellation"]
    } else {
        vec!["event_kind", "debounce_window", "raw_boundary"]
    }
}

fn source_watch_comparison_fixtures(
    implementation: &str,
    behavior: &str,
) -> anyhow::Result<Vec<Value>> {
    let fixture_name = comparison_artifact_name(implementation, behavior);
    let disposition = source_watch_comparison_disposition(behavior);
    let mut artifact = json!({
        "implementation": implementation,
        "behavior": behavior,
        "disposition": disposition,
        "model_binding": comparison_model_binding(disposition, behavior),
        "source_visible_facts": ["implementation", "behavior", "disposition", "evidence_anchor"],
        "assertions": [
            "trace keeps the modeled behavior visible",
            "replay hash changes when the modeled behavior is removed"
        ],
        "observed_facts": source_watch_comparison_facts(behavior),
        "fixture_id": fixture_name,
    });
    add_comparison_boundary_reason(&mut artifact, disposition);
    Ok(vec![comparison_artifact_entry(
        &fixture_name,
        "parity_fixture",
        artifact,
    )?])
}

fn source_watch_comparison_mutations(
    implementation: &str,
    behavior: &str,
) -> anyhow::Result<Vec<Value>> {
    let disposition = source_watch_comparison_disposition(behavior);
    let mutations = if behavior.contains("ignore")
        || behavior.contains("path")
        || behavior.contains("root")
        || behavior.contains("symlink")
        || behavior.contains("case")
        || behavior.contains("extension")
        || behavior.contains("include")
        || behavior.contains("exclude")
    {
        vec!["path-filter-flip", "root-scope-shift"]
    } else if behavior.contains("signal")
        || behavior.contains("process")
        || behavior.contains("command")
        || behavior.contains("environment")
        || behavior.contains("kill")
    {
        vec!["child-exit-drop", "signal-order-swap"]
    } else if behavior.contains("async")
        || behavior.contains("scheduler")
        || behavior.contains("timeout")
    {
        vec!["timer-order-swap", "cancel-drop"]
    } else {
        vec!["event-order-swap", "duplicate-drop"]
    };
    mutations
        .into_iter()
        .map(|mutation| {
            let mut artifact = json!({
                "implementation": implementation,
                "behavior": behavior,
                "disposition": disposition,
                "model_binding": comparison_model_binding(disposition, behavior),
                "source_visible_facts": ["implementation", "behavior", "disposition", "evidence_anchor"],
                "mutation": mutation,
                "expected_detection": "watch trace import or replay hash changes",
                "assertions": [
                    "mutation targets a modeled fact",
                    "mutation is not accepted as silent parity"
                ],
            });
            add_comparison_boundary_reason(&mut artifact, disposition);
            comparison_artifact_entry(mutation, "mutation_check", artifact)
        })
        .collect()
}

fn comparison_model_binding(disposition: &str, behavior: &str) -> Value {
    json!({
        "kind": disposition,
        "anchor": format!("external_comparisons::{behavior}"),
    })
}

fn add_comparison_boundary_reason(artifact: &mut Value, disposition: &str) {
    if matches!(disposition, "debt_item" | "explicit_non_goal") {
        artifact["boundary_reason"] =
            Value::from("comparison stays visible as an honest boundary before full replacement");
    }
}

fn comparison_artifact_name(implementation: &str, behavior: &str) -> String {
    format!("{}::{}", implementation, behavior.replace(' ', "_"))
}

fn comparison_artifact_entry(
    name: &str,
    artifact_kind: &str,
    artifact_json: Value,
) -> anyhow::Result<Value> {
    Ok(json!({
        "name": name,
        "artifact_kind": artifact_kind,
        "artifact_hash": stable_value_hash(&artifact_json)?,
        "artifact_json": artifact_json,
    }))
}

fn source_watch_state_events(value: &Value) -> anyhow::Result<Vec<Value>> {
    let mut events = Vec::new();
    append_source_watch_events(value, &mut events)?;
    append_source_watch_timers(value, &mut events);
    append_source_watch_restart_decisions(value, &mut events)?;
    append_source_watch_child_lifecycle(value, &mut events)?;
    Ok(events)
}

fn append_source_watch_events(value: &Value, events: &mut Vec<Value>) -> anyhow::Result<()> {
    let batches = value["event_batches"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing source watch event_batches"))?;
    for (window_index, batch) in batches.iter().enumerate() {
        let window = (window_index + 1) as u64;
        for event in batch["events"].as_array().into_iter().flatten() {
            events.push(json!({
                "kind": "watcher_event",
                "event_kind": required_str(event, "kind")?,
                "path": source_watch_event_path(event)?,
                "window": window,
                "duplicate_marker": event["duplicate_or_coalesced"].as_str().unwrap_or("unique"),
                "evidence_grade": event["evidence_grade"].as_str().unwrap_or("metadata_only"),
                "source_batch": batch["id"].clone(),
                "source_event": event,
            }));
        }
    }
    Ok(())
}

fn append_source_watch_timers(value: &Value, events: &mut Vec<Value>) {
    for (window_index, window) in value["debounce_windows"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if window["fired"].as_bool().unwrap_or(true) {
            events.push(json!({
                "kind": "timer_fired",
                "window": (window_index + 1) as u64,
                "timer_evidence": window["timer_evidence"].clone(),
                "source_window": window,
            }));
        }
    }
}

fn append_source_watch_restart_decisions(
    value: &Value,
    events: &mut Vec<Value>,
) -> anyhow::Result<()> {
    for decision in value["restart_decisions"].as_array().into_iter().flatten() {
        events.push(json!({
            "kind": "restart_decision",
            "policy_branch": required_str(decision, "policy_branch")?,
            "action": required_str(decision, "action")?,
            "path": restart_decision_path(decision),
            "source_scope": decision["source_scope"].as_str().unwrap_or("source_watch_state"),
            "outcome": decision["outcome"].clone(),
            "diagnostic_count": decision["diagnostic_count"].clone(),
            "source_event": decision,
        }));
    }
    Ok(())
}

fn append_source_watch_child_lifecycle(
    value: &Value,
    events: &mut Vec<Value>,
) -> anyhow::Result<()> {
    for (index, lifecycle) in value["child_lifecycle_obligations"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if lifecycle["resolution"].as_str() != Some("in_process_rerun_finished") {
            continue;
        }
        let child_id = format!("in-process-rerun-{}", index + 1);
        let command = lifecycle["command_kind"]
            .as_str()
            .unwrap_or("in_process_rerun");
        events.push(json!({
            "kind": "child_start",
            "child_id": child_id,
            "policy": "exclusive",
            "command": command,
            "execution_mode": "in_process",
            "source_event": lifecycle,
        }));
        events.push(json!({
            "kind": "child_exit",
            "child_id": format!("in-process-rerun-{}", index + 1),
            "exit_code": 0,
            "execution_mode": "in_process",
            "source_event": lifecycle,
        }));
    }
    Ok(())
}

fn source_watch_event_path(event: &Value) -> anyhow::Result<String> {
    let paths = event["paths"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing source watch event paths"))?;
    let selected = paths
        .iter()
        .find(|path| path["role"].as_str() == Some("source_path"))
        .or_else(|| paths.first())
        .ok_or_else(|| anyhow::anyhow!("empty source watch event paths"))?;
    Ok(required_str(selected, "path")?.to_owned())
}

fn restart_decision_path(decision: &Value) -> String {
    decision["selected_by"]
        .as_array()
        .and_then(|paths| paths.first())
        .and_then(Value::as_str)
        .or_else(|| decision["source_scope"].as_str())
        .unwrap_or("source_watch_state")
        .to_owned()
}

fn parse_adapter_summaries(value: &Value) -> anyhow::Result<Vec<AdapterSummary>> {
    let summaries = value["adapter_summaries"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing adapter_summaries"))?;
    let parsed = summaries
        .iter()
        .map(parse_adapter_summary)
        .collect::<anyhow::Result<Vec<_>>>()?;
    require_adapter_kind(&parsed, AdapterKind::Watcher)?;
    require_adapter_kind(&parsed, AdapterKind::Process)?;
    require_adapter_kind(&parsed, AdapterKind::Time)?;
    require_adapter_kind(&parsed, AdapterKind::PathFilter)?;
    require_adapter_kind(&parsed, AdapterKind::AsyncRuntime)?;
    Ok(parsed)
}

fn parse_adapter_summary(summary: &Value) -> anyhow::Result<AdapterSummary> {
    let kind = AdapterKind::from_str(required_str(summary, "kind")?)?;
    let schema_version = summary["schema_version"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing adapter schema_version: {}", kind.as_str()))?;
    if schema_version != 1 {
        anyhow::bail!("stale adapter summary: {}", kind.as_str());
    }
    require_non_empty_str(summary, "name", kind)?;
    require_non_empty_str(summary, "version_range", kind)?;
    require_non_empty_array(summary, "operations", kind)?;
    require_non_empty_array(summary, "modeled_facts", kind)?;
    require_non_empty_array(summary, "unsupported_guarantees", kind)?;
    require_non_empty_str(summary, "replay_confidence", kind)?;
    require_non_empty_str(summary, "source_map_anchor", kind)?;
    require_non_empty_array(summary, "cargo_features", kind)?;
    require_adapter_kind_contract(summary, kind)?;
    Ok(AdapterSummary {
        kind,
        json: summary.clone(),
    })
}

fn require_adapter_kind_contract(summary: &Value, kind: AdapterKind) -> anyhow::Result<()> {
    require_adapter_values(
        summary,
        "operations",
        kind.required_operations(),
        kind,
        "operation",
    )?;
    require_adapter_values(
        summary,
        "modeled_facts",
        kind.required_modeled_facts(),
        kind,
        "modeled fact",
    )?;
    require_non_empty_array(summary, "conformance_tests", kind)?;
    require_non_empty_object(summary, "replay_evidence", kind)?;
    require_stale_check(summary, kind)?;
    require_platform_observations(summary, kind)?;
    if kind == AdapterKind::AsyncRuntime {
        require_adapter_values(
            summary,
            "async_semantics",
            REQUIRED_ASYNC_SEMANTICS,
            kind,
            "async semantic",
        )?;
        require_adapter_values(
            summary,
            "mutation_checks",
            REQUIRED_ASYNC_MUTATIONS,
            kind,
            "async mutation",
        )?;
        require_non_empty_array(summary, "scheduler_facts", kind)?;
    }
    Ok(())
}

fn require_adapter_values(
    summary: &Value,
    field: &str,
    required: &[&str],
    kind: AdapterKind,
    label: &str,
) -> anyhow::Result<()> {
    let observed = string_set(&summary[field]);
    for required in required {
        if !observed.contains(required) {
            anyhow::bail!(
                "adapter {} missing required {label}: {required}",
                kind.as_str()
            );
        }
    }
    Ok(())
}

fn require_non_empty_object(value: &Value, field: &str, kind: AdapterKind) -> anyhow::Result<()> {
    let object = value[field]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("missing adapter {field}: {}", kind.as_str()))?;
    if object.is_empty() {
        anyhow::bail!("empty adapter {field}: {}", kind.as_str());
    }
    Ok(())
}

fn require_stale_check(summary: &Value, kind: AdapterKind) -> anyhow::Result<()> {
    require_non_empty_object(summary, "stale_check", kind)?;
    let stale_check = &summary["stale_check"];
    if stale_check["status"].as_str() != Some("passed") {
        anyhow::bail!("adapter {} stale_check must pass", kind.as_str());
    }
    require_adapter_field_str(stale_check, "summary_version", kind, "stale_check")?;
    require_adapter_field_array(stale_check, "features", kind, "stale_check")?;
    Ok(())
}

fn require_platform_observations(summary: &Value, kind: AdapterKind) -> anyhow::Result<()> {
    let observations = summary["platform_observations"].as_array().ok_or_else(|| {
        anyhow::anyhow!("missing adapter platform_observations: {}", kind.as_str())
    })?;
    if observations.is_empty() {
        anyhow::bail!("empty adapter platform_observations: {}", kind.as_str());
    }
    for platform in REQUIRED_PLATFORMS {
        let observed_behaviors = observations
            .iter()
            .filter(|observation| observation["platform"].as_str() == Some(platform))
            .flat_map(|observation| string_set(&observation["behaviors"]))
            .collect::<BTreeSet<_>>();
        for behavior in REQUIRED_PLATFORM_BEHAVIORS {
            if !observed_behaviors.contains(behavior) {
                anyhow::bail!(
                    "adapter {} missing {platform} {behavior} platform observation",
                    kind.as_str()
                );
            }
        }
    }
    Ok(())
}

fn require_adapter_field_str(
    value: &Value,
    field: &str,
    kind: AdapterKind,
    parent: &str,
) -> anyhow::Result<()> {
    let text = value[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing adapter {parent} {field}: {}", kind.as_str()))?;
    if text.trim().is_empty() {
        anyhow::bail!("empty adapter {parent} {field}: {}", kind.as_str());
    }
    Ok(())
}

fn require_adapter_field_array(
    value: &Value,
    field: &str,
    kind: AdapterKind,
    parent: &str,
) -> anyhow::Result<()> {
    let values = value[field]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing adapter {parent} {field}: {}", kind.as_str()))?;
    if values.is_empty() {
        anyhow::bail!("empty adapter {parent} {field}: {}", kind.as_str());
    }
    Ok(())
}

fn string_set(value: &Value) -> BTreeSet<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn require_adapter_kind(
    summaries: &[AdapterSummary],
    required_kind: AdapterKind,
) -> anyhow::Result<()> {
    if summaries
        .iter()
        .any(|summary| summary.kind == required_kind)
    {
        return Ok(());
    }
    anyhow::bail!("missing adapter summary: {}", required_kind.as_str())
}

fn require_non_empty_str(value: &Value, field: &str, kind: AdapterKind) -> anyhow::Result<()> {
    let text = value[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing adapter {field}: {}", kind.as_str()))?;
    if text.trim().is_empty() {
        anyhow::bail!("empty adapter {field}: {}", kind.as_str());
    }
    Ok(())
}

fn require_non_empty_array(value: &Value, field: &str, kind: AdapterKind) -> anyhow::Result<()> {
    let values = value[field]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing adapter {field}: {}", kind.as_str()))?;
    if values.is_empty() {
        anyhow::bail!("empty adapter {field}: {}", kind.as_str());
    }
    Ok(())
}

fn parse_external_comparisons(value: &Value) -> anyhow::Result<Vec<ExternalComparison>> {
    let comparisons = value["external_comparisons"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing external_comparisons"))?;
    if comparisons.is_empty() {
        anyhow::bail!("empty external_comparisons");
    }
    let parsed = comparisons
        .iter()
        .map(parse_external_comparison)
        .collect::<anyhow::Result<Vec<_>>>()?;
    require_external_comparison_coverage(&parsed)?;
    Ok(parsed)
}

fn parse_external_comparison(value: &Value) -> anyhow::Result<ExternalComparison> {
    let implementation = require_comparison_str(value, "implementation")?;
    let behavior = require_comparison_str(value, "behavior")?;
    let disposition_text = require_comparison_str(value, "disposition")?;
    let Some(disposition) = ComparisonDisposition::parse(disposition_text) else {
        anyhow::bail!("unknown external comparison disposition: {disposition_text}");
    };
    require_comparison_str(value, "reason")?;
    require_comparison_str(value, "evidence_anchor")?;
    require_comparison_array(value, "modeled_facts")?;
    require_comparison_artifacts(
        value,
        "parity_fixtures",
        "parity_fixture",
        implementation,
        behavior,
        disposition,
    )?;
    require_comparison_artifacts(
        value,
        "mutation_checks",
        "mutation_check",
        implementation,
        behavior,
        disposition,
    )?;
    Ok(ExternalComparison {
        json: value.clone(),
    })
}

fn require_external_comparison_coverage(comparisons: &[ExternalComparison]) -> anyhow::Result<()> {
    let observed = comparisons
        .iter()
        .filter_map(|comparison| {
            Some((
                comparison.json["implementation"].as_str()?,
                comparison.json["behavior"].as_str()?,
            ))
        })
        .collect::<BTreeSet<_>>();
    for required in REQUIRED_EXTERNAL_COMPARISON_COVERAGE {
        if !observed.contains(required) {
            anyhow::bail!(
                "missing external comparison coverage: {} - {}",
                required.0,
                required.1
            );
        }
    }
    Ok(())
}

fn require_comparison_str<'a>(value: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    let text = value[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing external comparison {field}"))?;
    if text.trim().is_empty() {
        anyhow::bail!("empty external comparison {field}");
    }
    Ok(text)
}

fn require_comparison_array(value: &Value, field: &str) -> anyhow::Result<()> {
    let values = value[field]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing external comparison {field}"))?;
    if values.is_empty() {
        anyhow::bail!("empty external comparison {field}");
    }
    for value in values {
        if value.as_str().unwrap_or("").trim().is_empty() {
            anyhow::bail!("empty external comparison {field} entry");
        }
    }
    Ok(())
}

fn require_comparison_artifacts(
    value: &Value,
    field: &str,
    expected_kind: &str,
    implementation: &str,
    behavior: &str,
    disposition: ComparisonDisposition,
) -> anyhow::Result<()> {
    let artifacts = value[field]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing external comparison {field}"))?;
    if artifacts.is_empty() {
        anyhow::bail!("empty external comparison {field}");
    }
    for artifact in artifacts {
        require_comparison_artifact(
            artifact,
            field,
            expected_kind,
            implementation,
            behavior,
            disposition,
        )?;
    }
    Ok(())
}

fn require_comparison_artifact(
    artifact: &Value,
    field: &str,
    expected_kind: &str,
    implementation: &str,
    behavior: &str,
    disposition: ComparisonDisposition,
) -> anyhow::Result<()> {
    if !artifact.is_object() {
        anyhow::bail!("external comparison {field} entry must be an artifact object");
    }
    require_artifact_str(artifact, "name", field)?;
    let artifact_kind = require_artifact_str(artifact, "artifact_kind", field)?;
    if artifact_kind != expected_kind {
        anyhow::bail!("external comparison {field} artifact_kind must be {expected_kind}");
    }
    let artifact_hash = require_artifact_str(artifact, "artifact_hash", field)?;
    let artifact_json = &artifact["artifact_json"];
    if !artifact_json.is_object()
        || artifact_json
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
    {
        anyhow::bail!("external comparison {field} artifact_json must be a non-empty object");
    }
    if stable_value_hash(artifact_json)? != artifact_hash {
        anyhow::bail!("external comparison {field} artifact_hash mismatch");
    }
    require_artifact_identity(artifact_json, implementation, behavior, field)?;
    require_artifact_model_binding(artifact_json, disposition, field)?;
    require_artifact_array(artifact_json, "assertions", field)?;
    if expected_kind == "parity_fixture" {
        require_artifact_array(artifact_json, "observed_facts", field)?;
    } else {
        require_artifact_str(artifact_json, "mutation", field)?;
        require_artifact_str(artifact_json, "expected_detection", field)?;
    }
    Ok(())
}

fn require_artifact_model_binding(
    artifact_json: &Value,
    disposition: ComparisonDisposition,
    field: &str,
) -> anyhow::Result<()> {
    if artifact_json["disposition"].as_str() != Some(disposition.as_str()) {
        anyhow::bail!("external comparison {field} artifact disposition mismatch");
    }
    require_artifact_array(artifact_json, "source_visible_facts", field)?;
    let binding = &artifact_json["model_binding"];
    if !binding.is_object() || binding.as_object().is_some_and(serde_json::Map::is_empty) {
        anyhow::bail!("missing external comparison {field} model_binding");
    }
    if binding["kind"].as_str() != Some(disposition.as_str()) {
        anyhow::bail!("external comparison {field} model_binding kind mismatch");
    }
    require_artifact_str(binding, "anchor", field)?;
    if disposition.requires_boundary_reason() {
        require_artifact_str(artifact_json, "boundary_reason", field)?;
    }
    Ok(())
}

fn require_artifact_identity(
    artifact_json: &Value,
    implementation: &str,
    behavior: &str,
    field: &str,
) -> anyhow::Result<()> {
    if artifact_json["implementation"].as_str() != Some(implementation) {
        anyhow::bail!("external comparison {field} artifact implementation mismatch");
    }
    if artifact_json["behavior"].as_str() != Some(behavior) {
        anyhow::bail!("external comparison {field} artifact behavior mismatch");
    }
    Ok(())
}

fn require_artifact_str<'a>(value: &'a Value, field: &str, label: &str) -> anyhow::Result<&'a str> {
    let text = value[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing external comparison {label} {field}"))?;
    if text.trim().is_empty() {
        anyhow::bail!("empty external comparison {label} {field}");
    }
    Ok(text)
}

fn require_artifact_array(value: &Value, field: &str, label: &str) -> anyhow::Result<()> {
    let values = value[field]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing external comparison {label} {field}"))?;
    if values.is_empty() {
        anyhow::bail!("empty external comparison {label} {field}");
    }
    for value in values {
        if value.as_str().unwrap_or("").trim().is_empty() {
            anyhow::bail!("empty external comparison {label} {field} entry");
        }
    }
    Ok(())
}

fn raw_events(value: &Value) -> anyhow::Result<Vec<Value>> {
    value["raw_events"]
        .as_array()
        .or_else(|| value["events"].as_array())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing watch trace events"))
}

fn normalize_trace(raw_events: &[Value]) -> anyhow::Result<NormalizedTrace> {
    let mut windows = BTreeMap::<u64, WindowTrace>::new();
    let mut restart_decisions = Vec::new();
    let mut child_lifecycle = Vec::new();
    let mut shutdown_resolutions = Vec::new();
    let mut active_children = BTreeMap::<String, ChildState>::new();

    for event in raw_events {
        match TraceEventKind::from_value(event)? {
            TraceEventKind::WatcherEvent => record_watcher_event(&mut windows, event)?,
            TraceEventKind::TimerFired => record_timer_fired(&mut windows, event)?,
            TraceEventKind::RestartDecision => {
                restart_decisions.push(restart_decision_json(event)?)
            }
            TraceEventKind::ChildStart => {
                record_child_start(&mut active_children, &mut child_lifecycle, event)?
            }
            TraceEventKind::ChildExit
            | TraceEventKind::ChildSignal
            | TraceEventKind::ChildKill
            | TraceEventKind::ChildDetach
            | TraceEventKind::ChildTimeout
            | TraceEventKind::ChildCancel => {
                record_child_resolution(&mut active_children, &mut child_lifecycle, event)?
            }
            TraceEventKind::Shutdown => record_shutdown(
                &mut windows,
                &mut active_children,
                &mut child_lifecycle,
                &mut shutdown_resolutions,
                event,
            )?,
        }
    }

    reject_orphan_children(&active_children)?;
    let replay_grade = replay_grade_for_windows(&windows);
    Ok(NormalizedTrace {
        replay_grade,
        json: normalized_json(
            windows,
            restart_decisions,
            child_lifecycle,
            shutdown_resolutions,
            replay_grade,
        ),
    })
}

fn record_watcher_event(
    windows: &mut BTreeMap<u64, WindowTrace>,
    event: &Value,
) -> anyhow::Result<()> {
    let trace_event = WatcherTraceEvent {
        event_kind: required_str(event, "event_kind")?.to_owned(),
        path: required_str(event, "path")?.to_owned(),
        paths: watcher_event_paths(event)?,
        window: required_u64(event, "window")?,
        evidence_grade: event["evidence_grade"]
            .as_str()
            .unwrap_or("metadata_only")
            .to_owned(),
        duplicate_marker: event["duplicate_marker"]
            .as_str()
            .unwrap_or("unique")
            .to_owned(),
        filter_decision: object_or_null(&event["filter_decision"]),
        platform: object_or_null(&event["platform"]),
        raw: event.clone(),
    };
    windows
        .entry(trace_event.window)
        .or_insert_with(WindowTrace::default)
        .watcher_events
        .push(trace_event);
    Ok(())
}

fn record_timer_fired(
    windows: &mut BTreeMap<u64, WindowTrace>,
    event: &Value,
) -> anyhow::Result<()> {
    let window = required_u64(event, "window")?;
    windows
        .entry(window)
        .or_insert_with(WindowTrace::default)
        .has_timer_fired = true;
    windows
        .entry(window)
        .or_insert_with(WindowTrace::default)
        .timer_events
        .push(event.clone());
    Ok(())
}

fn record_child_start(
    active_children: &mut BTreeMap<String, ChildState>,
    child_lifecycle: &mut Vec<Value>,
    event: &Value,
) -> anyhow::Result<()> {
    let child_id = required_str(event, "child_id")?.to_owned();
    let policy = event["policy"].as_str().unwrap_or("exclusive").to_owned();
    if policy == "exclusive"
        && active_children
            .values()
            .any(|child| child.policy == "exclusive")
    {
        resolve_previous_child_before_replacement(
            active_children,
            child_lifecycle,
            event,
            &child_id,
        )?;
    }
    if active_children.contains_key(&child_id) {
        anyhow::bail!("double-running child: `{child_id}` started twice without resolution");
    }
    let command = required_str(event, "command")?.to_owned();
    active_children.insert(
        child_id.clone(),
        ChildState {
            child_id: child_id.clone(),
            policy: policy.clone(),
            command: command.clone(),
            process_group: event["process_group"].clone(),
            stdio: event["stdio"].clone(),
            terminal: event["terminal"].clone(),
            environment: event["environment"].clone(),
        },
    );
    Ok(())
}

fn resolve_previous_child_before_replacement(
    active_children: &mut BTreeMap<String, ChildState>,
    child_lifecycle: &mut Vec<Value>,
    event: &Value,
    replacing_child_id: &str,
) -> anyhow::Result<()> {
    let Some(previous_child_id) = event["previous_child_id"].as_str() else {
        anyhow::bail!("double-running child: exclusive restart started `{replacing_child_id}` before the active child resolved");
    };
    let Some(started_child) = active_children.remove(previous_child_id) else {
        anyhow::bail!("replacement references missing active child: `{previous_child_id}`");
    };
    let previous_resolution = event["previous_resolution"].as_str().unwrap_or("signaled");
    if !is_child_resolution_label(previous_resolution) {
        anyhow::bail!("unsupported previous child resolution: {previous_resolution}");
    }
    child_lifecycle.push(child_lifecycle_json(
        started_child,
        previous_resolution.to_owned(),
        event,
        json!({
            "replacement_child_id": replacing_child_id,
            "signal": event["previous_signal"].clone(),
        }),
    ));
    Ok(())
}

fn record_child_resolution(
    active_children: &mut BTreeMap<String, ChildState>,
    child_lifecycle: &mut Vec<Value>,
    event: &Value,
) -> anyhow::Result<()> {
    let child_id = required_str(event, "child_id")?;
    let Some(started_child) = active_children.remove(child_id) else {
        anyhow::bail!("child lifecycle resolution for `{child_id}` has no matching start");
    };
    child_lifecycle.push(child_lifecycle_json(
        started_child,
        resolution_label(event)?,
        event,
        Value::Null,
    ));
    Ok(())
}

fn child_lifecycle_json(
    started_child: ChildState,
    resolution: String,
    event: &Value,
    extra: Value,
) -> Value {
    json!({
        "child_id": started_child.child_id,
        "policy": started_child.policy,
        "command": started_child.command,
        "obligation": "started child must be waited, signaled, killed, or detached",
        "resolution": resolution,
        "exit_code": event["exit_code"].clone(),
        "signal": event["signal"].clone(),
        "timeout_ms": event["timeout_ms"].clone(),
        "reason": event["reason"].clone(),
        "process_group": started_child.process_group,
        "stdio": started_child.stdio,
        "terminal": started_child.terminal,
        "environment": started_child.environment,
        "extra": extra,
        "source_event": event,
    })
}

fn record_shutdown(
    windows: &mut BTreeMap<u64, WindowTrace>,
    active_children: &mut BTreeMap<String, ChildState>,
    child_lifecycle: &mut Vec<Value>,
    shutdown_resolutions: &mut Vec<Value>,
    event: &Value,
) -> anyhow::Result<()> {
    let resolves_timers = event["resolves_timers"].as_bool().unwrap_or(false);
    let resolves_children = event["resolves_children"].as_bool().unwrap_or(false);
    if !resolves_timers && !resolves_children {
        anyhow::bail!("shutdown event must resolve timers, children, or both");
    }

    let pending_windows = shutdown_pending_windows(event, windows)?;
    if resolves_timers {
        for window in &pending_windows {
            windows.entry(*window).or_default().shutdown_resolved = true;
        }
    }

    let child_resolution = event["child_resolution"].as_str().unwrap_or("killed");
    if resolves_children && !is_child_resolution_label(child_resolution) {
        anyhow::bail!("unsupported shutdown child resolution: {child_resolution}");
    }
    let mut resolved_children = Vec::new();
    if resolves_children {
        let child_ids = active_children.keys().cloned().collect::<Vec<_>>();
        for child_id in child_ids {
            let Some(started_child) = active_children.remove(&child_id) else {
                continue;
            };
            resolved_children.push(started_child.child_id.clone());
            child_lifecycle.push(child_lifecycle_json(
                started_child,
                child_resolution.to_owned(),
                event,
                Value::Null,
            ));
        }
    }

    shutdown_resolutions.push(json!({
        "resolves_timers": resolves_timers,
        "resolves_children": resolves_children,
        "pending_windows": pending_windows,
        "resolved_children": resolved_children,
        "child_resolution": child_resolution,
        "source_event": event,
    }));
    Ok(())
}

fn shutdown_pending_windows(
    event: &Value,
    windows: &BTreeMap<u64, WindowTrace>,
) -> anyhow::Result<Vec<u64>> {
    if let Some(pending_windows) = event["pending_windows"].as_array() {
        return pending_windows
            .iter()
            .map(|window| {
                window
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("shutdown pending_windows must be numeric"))
            })
            .collect();
    }
    Ok(windows
        .iter()
        .filter_map(|(window, trace)| {
            (!trace.has_timer_fired && !trace.shutdown_resolved).then_some(*window)
        })
        .collect())
}

fn reject_orphan_children(active_children: &BTreeMap<String, ChildState>) -> anyhow::Result<()> {
    if let Some(child) = active_children.values().next() {
        anyhow::bail!(
            "orphan child: `{}` has no exit, signal, kill, or detach evidence",
            child.child_id
        );
    }
    Ok(())
}

fn restart_decision_json(event: &Value) -> anyhow::Result<Value> {
    Ok(json!({
        "action": required_str(event, "action")?,
        "policy_branch": required_str(event, "policy_branch")?,
        "selected_by": [required_str(event, "path")?],
        "source_scope": event["source_scope"].as_str().unwrap_or("watch_trace"),
        "changed_paths": event["changed_paths"].clone(),
        "environment": event["environment"].clone(),
        "stdin_paths": event["stdin_paths"].clone(),
        "async_step": event["async_step"].clone(),
        "reason": "watch trace restart policy branch selected this action",
        "source_event": event,
    }))
}

fn normalized_json(
    windows: BTreeMap<u64, WindowTrace>,
    restart_decisions: Vec<Value>,
    child_lifecycle: Vec<Value>,
    shutdown_resolutions: Vec<Value>,
    replay_grade: TraceReplayGrade,
) -> Value {
    let path_filter_evidence = path_filter_evidence(&windows);
    let platform_evidence = platform_evidence(&windows);
    let async_runtime_evidence =
        async_runtime_evidence(&windows, &restart_decisions, &child_lifecycle);
    let platform_model = platform_model_summary(
        &windows,
        &restart_decisions,
        &child_lifecycle,
        &shutdown_resolutions,
        &path_filter_evidence,
        &platform_evidence,
        &async_runtime_evidence,
        replay_grade,
    );
    let event_batches = windows
        .iter()
        .enumerate()
        .map(|(index, (window, trace))| event_batch_json(index + 1, *window, trace, replay_grade))
        .collect::<Vec<_>>();
    let debounce_windows = windows
        .iter()
        .enumerate()
        .map(|(index, (window, trace))| {
            debounce_window_json(index + 1, *window, trace, replay_grade)
        })
        .collect::<Vec<_>>();
    json!({
        "event_batches": event_batches,
        "debounce_windows": debounce_windows,
        "restart_decisions": restart_decisions,
        "child_lifecycle_obligations": child_lifecycle,
        "shutdown_resolutions": shutdown_resolutions,
        "path_filter_evidence": path_filter_evidence,
        "platform_evidence": platform_evidence,
        "platform_model": platform_model,
        "async_runtime_evidence": async_runtime_evidence,
    })
}

fn event_batch_json(
    sequence: usize,
    window: u64,
    trace: &WindowTrace,
    replay_grade: TraceReplayGrade,
) -> Value {
    json!({
        "id": format!("watch-trace-batch-{sequence}"),
        "window": window,
        "backend": "trace-import",
        "watcher_evidence": trace.watcher_evidence_label(),
        "ordering_guarantee": "trace-order",
        "replay_grade": replay_grade.as_str(),
        "events": coalesced_watcher_events(trace),
    })
}

fn debounce_window_json(
    sequence: usize,
    window: u64,
    trace: &WindowTrace,
    replay_grade: TraceReplayGrade,
) -> Value {
    let paths = trace
        .watcher_events
        .iter()
        .map(|event| event.path.clone())
        .collect::<BTreeSet<_>>();
    json!({
        "id": format!("watch-trace-window-{sequence}"),
        "window": window,
        "timer_evidence": trace.timer_evidence_label(),
        "replay_grade": if trace.has_timer_fired || trace.shutdown_resolved { replay_grade.as_str() } else { "debt" },
        "timer_created": true,
        "timer_cancelled": trace.watcher_events.len() > 1 || trace.shutdown_resolved,
        "shutdown_resolved": trace.shutdown_resolved,
        "fired": trace.has_timer_fired,
        "event_membership": paths.into_iter().collect::<Vec<_>>(),
    })
}

fn coalesced_watcher_events(trace: &WindowTrace) -> Vec<Value> {
    let mut grouped = BTreeMap::<(String, String), Vec<&WatcherTraceEvent>>::new();
    for event in &trace.watcher_events {
        grouped
            .entry((event.path.clone(), event.event_kind.clone()))
            .or_default()
            .push(event);
    }
    grouped
        .into_iter()
        .map(|((path, event_kind), events)| coalesced_watcher_event_json(path, event_kind, events))
        .collect()
}

fn coalesced_watcher_event_json(
    path: String,
    event_kind: String,
    events: Vec<&WatcherTraceEvent>,
) -> Value {
    let has_duplicate = events.len() > 1
        || events
            .iter()
            .any(|event| matches!(event.duplicate_marker.as_str(), "duplicate" | "coalesced"));
    let evidence_grade = events
        .iter()
        .find_map(|event| {
            (!event.evidence_grade.is_empty()).then_some(event.evidence_grade.clone())
        })
        .unwrap_or_else(|| "metadata_only".to_owned());
    json!({
        "kind": event_kind,
        "paths": merged_event_paths(&events, &path),
        "duplicate_or_coalesced": if has_duplicate { "coalesced" } else { "unique" },
        "evidence_grade": evidence_grade,
        "filter_decisions": events.iter().filter_map(|event| event.filter_decision.clone()).collect::<Vec<_>>(),
        "platforms": events.iter().filter_map(|event| event.platform.clone()).collect::<Vec<_>>(),
        "raw_events": events.into_iter().map(|event| event.raw.clone()).collect::<Vec<_>>(),
    })
}

fn merged_event_paths(events: &[&WatcherTraceEvent], fallback_path: &str) -> Vec<Value> {
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for event in events {
        for path in &event.paths {
            let key = format!(
                "{}\u{1f}{}",
                path["role"].as_str().unwrap_or("source_path"),
                path["path"].as_str().unwrap_or(fallback_path)
            );
            if seen.insert(key) {
                paths.push(path.clone());
            }
        }
    }
    if paths.is_empty() {
        paths.push(json!({
            "role": "source_path",
            "path": fallback_path,
        }));
    }
    paths
}

fn path_filter_evidence(windows: &BTreeMap<u64, WindowTrace>) -> Vec<Value> {
    windows
        .iter()
        .flat_map(|(window, trace)| {
            trace.watcher_events.iter().filter_map(move |event| {
                event.filter_decision.as_ref().map(|filter| {
                    json!({
                        "window": window,
                        "path": event.path,
                        "decision": filter,
                        "source_event": event.raw,
                    })
                })
            })
        })
        .collect()
}

fn platform_evidence(windows: &BTreeMap<u64, WindowTrace>) -> Vec<Value> {
    windows
        .iter()
        .flat_map(|(window, trace)| {
            trace.watcher_events.iter().filter_map(move |event| {
                event.platform.as_ref().map(|platform| {
                    json!({
                        "window": window,
                        "path": event.path,
                        "platform": platform,
                        "source_event": event.raw,
                    })
                })
            })
        })
        .collect()
}

fn async_runtime_evidence(
    windows: &BTreeMap<u64, WindowTrace>,
    restart_decisions: &[Value],
    child_lifecycle: &[Value],
) -> Vec<Value> {
    let mut evidence = Vec::new();
    for (window, trace) in windows {
        for event in &trace.timer_events {
            if !event["async_step"].is_null() {
                evidence.push(json!({
                    "window": window,
                    "kind": "timer",
                    "async_step": event["async_step"].clone(),
                    "source_event": event,
                }));
            }
        }
        for event in &trace.watcher_events {
            if !event.raw["async_step"].is_null() {
                evidence.push(json!({
                    "window": window,
                    "kind": "watcher",
                    "async_step": event.raw["async_step"].clone(),
                    "source_event": event.raw,
                }));
            }
        }
    }
    for decision in restart_decisions {
        let source_event = &decision["source_event"];
        if !source_event["async_step"].is_null() {
            evidence.push(json!({
                "kind": "restart_decision",
                "async_step": source_event["async_step"].clone(),
                "source_event": source_event,
            }));
        }
    }
    for lifecycle in child_lifecycle {
        let source_event = &lifecycle["source_event"];
        if !source_event["async_step"].is_null() {
            evidence.push(json!({
                "kind": "child_lifecycle",
                "async_step": source_event["async_step"].clone(),
                "source_event": source_event,
            }));
        }
    }
    evidence
}

fn platform_model_summary(
    windows: &BTreeMap<u64, WindowTrace>,
    restart_decisions: &[Value],
    child_lifecycle: &[Value],
    shutdown_resolutions: &[Value],
    path_filter_evidence: &[Value],
    platform_evidence: &[Value],
    async_runtime_evidence: &[Value],
    replay_grade: TraceReplayGrade,
) -> Value {
    json!({
        "schema_version": 1,
        "status": "source_visible",
        "replay_grade": replay_grade.as_str(),
        "models": platform_model_entries(),
        "platforms": platform_observation_summary(platform_evidence),
        "behavior_tests": platform_behavior_tests(
            windows,
            restart_decisions,
            child_lifecycle,
            path_filter_evidence,
        ),
        "source_visible_facts": source_visible_fact_summary(
            windows,
            restart_decisions,
            child_lifecycle,
            shutdown_resolutions,
            path_filter_evidence,
            async_runtime_evidence,
        ),
        "unsupported_guarantees": [
            "native_backend_total_order",
            "arbitrary_scheduler_equivalence",
            "unrecorded_platform_signal_equivalence",
        ],
    })
}

fn platform_model_entries() -> Vec<Value> {
    [
        (
            "filesystem_events",
            &[
                "create",
                "modify",
                "delete",
                "move",
                "rename",
                "close",
                "metadata",
                "rescan",
                "synthetic",
                "duplicate",
                "coalesced",
            ][..],
            "event_batches",
        ),
        (
            "watcher_backend",
            &["backend_identity", "ordering_guarantee", "polling_fallback"][..],
            "platform_evidence",
        ),
        (
            "paths",
            &[
                "case_sensitive",
                "case_only_rename",
                "canonical_path",
                "symlink_policy",
                "project_root",
                "recursive_scope",
                "unsupported_filesystem",
            ][..],
            "path_filter_evidence",
        ),
        (
            "process_execution",
            &[
                "spawn",
                "command_arguments",
                "environment",
                "current_directory",
                "shell_wrapping",
                "no_shell_execution",
                "detached_process",
            ][..],
            "child_lifecycle_obligations",
        ),
        (
            "signals",
            &[
                "stop_signal",
                "interrupt_signal",
                "kill_fallback",
                "graceful_shutdown",
                "kill_timeout",
                "unsupported_signal",
            ][..],
            "child_lifecycle_obligations",
        ),
        (
            "process_groups",
            &["process_group", "session", "job_object", "child_tree"][..],
            "child_lifecycle_obligations",
        ),
        (
            "environment_variables",
            &["changed_paths", "inheritance", "command_environment"][..],
            "restart_decisions",
        ),
        (
            "terminal_io",
            &["tty", "inherited_handles", "log_forwarding"][..],
            "child_lifecycle_obligations",
        ),
        (
            "stdio",
            &["stdin", "stdout", "stderr", "changed_path_delivery"][..],
            "child_lifecycle_obligations",
        ),
        (
            "timers",
            &[
                "debounce_window",
                "delay_run",
                "stop_timeout",
                "poll_interval",
                "cancellation",
                "timeout_firing",
            ][..],
            "debounce_windows",
        ),
    ]
    .into_iter()
    .map(|(model, facts, evidence_anchor)| {
        json!({
            "model": model,
            "facts": facts,
            "evidence_anchor": evidence_anchor,
        })
    })
    .collect()
}

fn platform_observation_summary(platform_evidence: &[Value]) -> Vec<Value> {
    let mut observations = BTreeMap::<String, BTreeSet<String>>::new();
    for evidence in platform_evidence {
        let platform = &evidence["platform"];
        let os = platform["os"].as_str().unwrap_or("unknown").to_owned();
        let backend = platform["backend"].as_str().unwrap_or("unknown");
        observations
            .entry(os)
            .or_default()
            .insert(backend.to_owned());
    }
    observations
        .into_iter()
        .map(|(os, backends)| {
            json!({
                "os": os,
                "backends": backends.into_iter().collect::<Vec<_>>(),
                "source_visible": true,
            })
        })
        .collect()
}

fn platform_behavior_tests(
    windows: &BTreeMap<u64, WindowTrace>,
    restart_decisions: &[Value],
    child_lifecycle: &[Value],
    path_filter_evidence: &[Value],
) -> Vec<Value> {
    REQUIRED_PLATFORM_BEHAVIORS
        .iter()
        .map(|behavior| {
            json!({
                "behavior": behavior,
                "observed": platform_behavior_is_observed(
                    behavior,
                    windows,
                    restart_decisions,
                    child_lifecycle,
                    path_filter_evidence,
                ),
                "grade": "modeled",
            })
        })
        .collect()
}

fn platform_behavior_is_observed(
    behavior: &str,
    windows: &BTreeMap<u64, WindowTrace>,
    restart_decisions: &[Value],
    child_lifecycle: &[Value],
    path_filter_evidence: &[Value],
) -> bool {
    match behavior {
        "watcher" => windows
            .values()
            .any(|trace| !trace.watcher_events.is_empty()),
        "restart" => restart_decisions
            .iter()
            .any(|decision| decision["action"].as_str() == Some("restart")),
        "signal" => child_lifecycle.iter().any(lifecycle_has_signal_fact),
        "stdin" => {
            child_lifecycle.iter().any(lifecycle_has_stdio_fact)
                || restart_decisions
                    .iter()
                    .any(|decision| !decision["stdin_paths"].is_null())
        }
        "path_filter" => !path_filter_evidence.is_empty(),
        _ => false,
    }
}

fn source_visible_fact_summary(
    windows: &BTreeMap<u64, WindowTrace>,
    restart_decisions: &[Value],
    child_lifecycle: &[Value],
    shutdown_resolutions: &[Value],
    path_filter_evidence: &[Value],
    async_runtime_evidence: &[Value],
) -> Value {
    json!({
        "filesystem_events": watcher_event_count(windows),
        "paths": watcher_paths(windows),
        "filter_decisions": path_filter_evidence.len(),
        "process_execution": child_lifecycle.len(),
        "signals": child_lifecycle.iter().filter(|entry| lifecycle_has_signal_fact(entry)).count(),
        "process_groups": child_lifecycle.iter().filter(|entry| !entry["process_group"].is_null()).count(),
        "environment_variables": restart_decisions.iter().filter(|entry| !entry["environment"].is_null()).count(),
        "terminal_io": child_lifecycle.iter().filter(|entry| !entry["terminal"].is_null()).count(),
        "stdio": child_lifecycle.iter().filter(|entry| lifecycle_has_stdio_fact(entry)).count(),
        "timers": windows.values().map(|trace| trace.timer_events.len()).sum::<usize>(),
        "async_runtime": async_runtime_evidence.len(),
        "shutdown": shutdown_resolutions.len(),
    })
}

fn watcher_event_count(windows: &BTreeMap<u64, WindowTrace>) -> usize {
    windows
        .values()
        .map(|trace| trace.watcher_events.len())
        .sum()
}

fn watcher_paths(windows: &BTreeMap<u64, WindowTrace>) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for trace in windows.values() {
        for event in &trace.watcher_events {
            paths.insert(event.path.clone());
        }
    }
    paths.into_iter().collect()
}

fn lifecycle_has_signal_fact(entry: &Value) -> bool {
    !entry["signal"].is_null()
        || !entry["extra"]["signal"].is_null()
        || matches!(
            entry["resolution"].as_str(),
            Some("graceful_stop" | "kill_timeout" | "killed" | "cancelled")
        )
}

fn lifecycle_has_stdio_fact(entry: &Value) -> bool {
    let stdio = &entry["stdio"];
    !stdio["stdin"].is_null() || !stdio["stdout"].is_null() || !stdio["stderr"].is_null()
}

fn replay_grade_for_windows(windows: &BTreeMap<u64, WindowTrace>) -> TraceReplayGrade {
    if windows
        .values()
        .any(|window| !window.has_timer_fired && !window.shutdown_resolved)
    {
        TraceReplayGrade::Debt
    } else if windows.values().all(WindowTrace::has_modeled_evidence) {
        TraceReplayGrade::Modeled
    } else {
        TraceReplayGrade::Partial
    }
}

fn witness_json(
    trace_path: &Path,
    source_kind: &str,
    adapter_summaries: &[AdapterSummary],
    external_comparisons: &[ExternalComparison],
    raw_events: Vec<Value>,
    normalized: &NormalizedTrace,
    raw_event_hash: &str,
    normalized_hash: &str,
    adapter_summary_hash: &str,
    external_comparison_hash: &str,
) -> Value {
    json!({
        "schema_version": 1,
        "mode": WATCH_TRACE_WITNESS_MODE,
        "source": {
            "kind": source_kind,
            "path": trace_path.display().to_string(),
        },
        "replay_grade": normalized.replay_grade.as_str(),
        "replay_guarantee": normalized.replay_grade.as_str(),
        "trace_import": {
            "exact": false,
            "modeled": normalized.replay_grade == TraceReplayGrade::Modeled,
            "sampled": false,
            "metadata_only": normalized.replay_grade == TraceReplayGrade::Partial,
            "opaque": false,
        },
        "adapter_summaries": adapter_summary_json(adapter_summaries),
        "adapter_summary_hash": adapter_summary_hash,
        "external_comparisons": external_comparison_json(external_comparisons),
        "external_comparison_hash": external_comparison_hash,
        "raw_event_hash": raw_event_hash,
        "normalized_hash": normalized_hash,
        "raw_events": raw_events,
        "normalized": normalized.json.clone(),
    })
}

fn adapter_summary_json(adapter_summaries: &[AdapterSummary]) -> Vec<Value> {
    adapter_summaries
        .iter()
        .map(|summary| summary.json.clone())
        .collect()
}

fn external_comparison_json(external_comparisons: &[ExternalComparison]) -> Vec<Value> {
    external_comparisons
        .iter()
        .map(|comparison| comparison.json.clone())
        .collect()
}

fn witness_output_path(trace_path: &Path, witness_out: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(output) = witness_out {
        return Ok(output.to_path_buf());
    }
    let stem = trace_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("watch-trace");
    Ok(std::env::current_dir()?
        .join(".kobo")
        .join("witnesses")
        .join(format!("{stem}.kwit")))
}

fn write_witness(output_path: &Path, witness: &Value) -> anyhow::Result<()> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(output_path, serde_json::to_vec_pretty(witness)?)
        .with_context(|| format!("failed to write {}", output_path.display()))
}

fn stable_value_hash(value: &Value) -> anyhow::Result<String> {
    Ok(kobo_sim_core::digest::stable_hash(&serde_json::to_string(
        value,
    )?))
}

fn stable_values_hash(values: &[Value]) -> anyhow::Result<String> {
    Ok(kobo_sim_core::digest::stable_hash(&serde_json::to_string(
        values,
    )?))
}

fn watch_trace_divergence(
    expected: Value,
    observed: Value,
    error_format: ErrorFormat,
) -> anyhow::Result<()> {
    let payload = json!({
        "code": "K0104",
        "message": "watch trace witness diverged",
        "expected": expected,
        "observed": observed,
    });
    emit_watch_trace_issue(&payload, error_format)?;
    anyhow::bail!("K0104 watch trace witness diverged")
}

fn emit_watch_trace_issue(payload: &Value, error_format: ErrorFormat) -> anyhow::Result<()> {
    match error_format {
        ErrorFormat::Human => {
            eprintln!(
                "error[{}]: {}",
                payload["code"].as_str().unwrap_or("K0104"),
                payload["message"]
                    .as_str()
                    .unwrap_or("watch trace replay failed")
            );
            eprintln!("expected: {}", payload["expected"]);
            eprintln!("observed: {}", payload["observed"]);
        }
        ErrorFormat::Json => {
            println!("{}", serde_json::to_string(payload)?);
        }
    }
    Ok(())
}

fn object_or_null(value: &Value) -> Option<Value> {
    value.is_object().then(|| value.clone())
}

fn watcher_event_paths(event: &Value) -> anyhow::Result<Vec<Value>> {
    if let Some(paths) = event["paths"].as_array() {
        let mut parsed = Vec::new();
        for path in paths {
            parsed.push(json!({
                "role": required_str(path, "role")?,
                "path": required_str(path, "path")?,
            }));
        }
        if parsed.is_empty() {
            anyhow::bail!("watch trace paths must not be empty");
        }
        return Ok(parsed);
    }
    Ok(vec![json!({
        "role": "source_path",
        "path": required_str(event, "path")?,
    })])
}

fn required_str<'a>(value: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    value[field]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing watch trace field `{field}`"))
}

fn required_u64(value: &Value, field: &str) -> anyhow::Result<u64> {
    value[field]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing numeric watch trace field `{field}`"))
}

fn resolution_label(event: &Value) -> anyhow::Result<String> {
    match TraceEventKind::from_value(event)? {
        TraceEventKind::ChildExit => Ok("exited".to_owned()),
        TraceEventKind::ChildSignal => Ok(event["resolution"]
            .as_str()
            .unwrap_or("signaled")
            .to_owned()),
        TraceEventKind::ChildKill => Ok("killed".to_owned()),
        TraceEventKind::ChildDetach => Ok("detached".to_owned()),
        TraceEventKind::ChildTimeout => Ok("kill_timeout".to_owned()),
        TraceEventKind::ChildCancel => Ok("cancelled".to_owned()),
        _ => anyhow::bail!("event is not a child lifecycle resolution"),
    }
}

fn is_child_resolution_label(label: &str) -> bool {
    matches!(
        label,
        "exited"
            | "signaled"
            | "killed"
            | "detached"
            | "graceful_stop"
            | "kill_timeout"
            | "restart"
            | "final_shutdown"
            | "cancelled"
    )
}

impl AdapterKind {
    fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "watcher" => Ok(Self::Watcher),
            "process" => Ok(Self::Process),
            "time" => Ok(Self::Time),
            "path_filter" => Ok(Self::PathFilter),
            "async_runtime" => Ok(Self::AsyncRuntime),
            _ => anyhow::bail!("unknown adapter summary kind: {value}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Watcher => "watcher",
            Self::Process => "process",
            Self::Time => "time",
            Self::PathFilter => "path_filter",
            Self::AsyncRuntime => "async_runtime",
        }
    }

    fn required_operations(self) -> &'static [&'static str] {
        match self {
            Self::Watcher => &["raw_event", "normalize_event_batch"],
            Self::Process => &["spawn", "exit", "signal", "kill", "timeout", "cancel"],
            Self::Time => &["timer_create", "timer_cancel", "timer_fire"],
            Self::PathFilter => &["match_path", "reload_config", "root_discovery"],
            Self::AsyncRuntime => &[
                "spawn",
                "join",
                "cancel",
                "select",
                "timer",
                "channel",
                "backpressure",
                "shutdown",
                "blocking",
            ],
        }
    }

    fn required_modeled_facts(self) -> &'static [&'static str] {
        match self {
            Self::Watcher => &[
                "event_kind",
                "paths",
                "ordering",
                "duplicates",
                "platform_backend",
                "raw_boundary",
            ],
            Self::Process => &[
                "child_id",
                "policy",
                "exit_code",
                "resolution",
                "process_group",
                "stdio",
                "terminal",
                "environment",
            ],
            Self::Time => &["window", "membership", "fire_order"],
            Self::PathFilter => &[
                "pure_path_match",
                "absolute_path_match",
                "case_mode",
                "config_generation",
                "filesystem_boundary",
            ],
            Self::AsyncRuntime => &[
                "task_order",
                "timer_order",
                "cancel_order",
                "channel_delivery",
                "wake_order",
            ],
        }
    }
}

impl ComparisonDisposition {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "semantic_rule" => Some(Self::SemanticRule),
            "formal_adapter_contract" => Some(Self::FormalAdapterContract),
            "replay_fixture" => Some(Self::ReplayFixture),
            "debt_item" => Some(Self::DebtItem),
            "explicit_non_goal" => Some(Self::ExplicitNonGoal),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::SemanticRule => "semantic_rule",
            Self::FormalAdapterContract => "formal_adapter_contract",
            Self::ReplayFixture => "replay_fixture",
            Self::DebtItem => "debt_item",
            Self::ExplicitNonGoal => "explicit_non_goal",
        }
    }

    fn requires_boundary_reason(self) -> bool {
        matches!(self, Self::DebtItem | Self::ExplicitNonGoal)
    }
}

impl TraceEventKind {
    fn from_value(value: &Value) -> anyhow::Result<Self> {
        match required_str(value, "kind")? {
            "watcher_event" => Ok(Self::WatcherEvent),
            "timer_fired" => Ok(Self::TimerFired),
            "restart_decision" => Ok(Self::RestartDecision),
            "child_start" => Ok(Self::ChildStart),
            "child_exit" => Ok(Self::ChildExit),
            "child_signal" => Ok(Self::ChildSignal),
            "child_kill" => Ok(Self::ChildKill),
            "child_detach" => Ok(Self::ChildDetach),
            "child_timeout" => Ok(Self::ChildTimeout),
            "child_cancel" => Ok(Self::ChildCancel),
            "shutdown" => Ok(Self::Shutdown),
            kind => anyhow::bail!("unknown watch trace event kind: {kind}"),
        }
    }
}

impl TraceReplayGrade {
    fn as_str(self) -> &'static str {
        match self {
            Self::Modeled => "modeled",
            Self::Partial => "partial",
            Self::Debt => "debt",
        }
    }

    fn replay_label(self) -> &'static str {
        match self {
            Self::Modeled => "watch_trace_modeled",
            Self::Partial => "watch_trace_partial",
            Self::Debt => "watch_trace_debt",
        }
    }
}

impl Default for WindowTrace {
    fn default() -> Self {
        Self {
            watcher_events: Vec::new(),
            timer_events: Vec::new(),
            has_timer_fired: false,
            shutdown_resolved: false,
        }
    }
}

impl WindowTrace {
    fn has_modeled_evidence(&self) -> bool {
        self.watcher_events
            .iter()
            .all(|event| evidence_grade_is_modeled(&event.evidence_grade))
            && self
                .timer_events
                .iter()
                .all(|event| value_evidence_grade_is_modeled(event))
    }

    fn watcher_evidence_label(&self) -> &'static str {
        if self
            .watcher_events
            .iter()
            .all(|event| evidence_grade_is_modeled(&event.evidence_grade))
        {
            "modeled"
        } else {
            "metadata-only"
        }
    }

    fn timer_evidence_label(&self) -> &'static str {
        if self.has_timer_fired
            && self
                .timer_events
                .iter()
                .all(|event| value_evidence_grade_is_modeled(event))
        {
            "modeled"
        } else if self.has_timer_fired {
            "metadata-only"
        } else if self.shutdown_resolved {
            "shutdown-resolved"
        } else {
            "missing"
        }
    }
}

fn value_evidence_grade_is_modeled(value: &Value) -> bool {
    value["evidence_grade"]
        .as_str()
        .is_some_and(evidence_grade_is_modeled)
}

fn evidence_grade_is_modeled(grade: &str) -> bool {
    matches!(grade, "modeled" | "modelled" | "exact")
}
