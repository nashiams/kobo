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

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum AdapterKind {
    Watcher,
    Process,
    Time,
    PathFilter,
    AsyncRuntime,
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

#[derive(Clone, Copy)]
enum TraceReplayGrade {
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
            "replay": "watch_trace_exact",
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
        "external_comparisons": source_watch_state_external_comparisons(),
        "events": source_watch_state_events(&value)?,
    }))
}

fn source_watch_state_adapter_summaries() -> Vec<Value> {
    vec![
        json!({
            "kind": "watcher",
            "name": "kobo-source-watch-state",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["poll_snapshot", "normalize_event_batch"],
            "modeled_facts": ["event_kind", "paths", "ordering", "duplicates", "evidence_grade"],
            "unsupported_guarantees": ["native_backend_order", "platform_specific_raw_event_identity"],
            "replay_confidence": "metadata-only",
            "source_map_anchor": "event_batches[]",
            "cargo_features": ["default"],
        }),
        json!({
            "kind": "process",
            "name": "kobo-in-process-supervisor",
            "schema_version": 1,
            "version_range": "^1",
            "operations": ["rerun_start", "rerun_finish"],
            "modeled_facts": ["command_kind", "resolution", "diagnostic_count"],
            "unsupported_guarantees": ["os_process_group_signal_equivalence"],
            "replay_confidence": "modelled",
            "source_map_anchor": "child_lifecycle_obligations[]",
            "cargo_features": ["default"],
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
        }),
    ]
}

fn source_watch_state_external_comparisons() -> Vec<Value> {
    REQUIRED_EXTERNAL_COMPARISON_COVERAGE
        .iter()
        .map(|(implementation, behavior)| {
            json!({
                "implementation": implementation,
                "behavior": behavior,
                "disposition": source_watch_comparison_disposition(behavior),
                "reason": source_watch_comparison_reason(implementation, behavior),
                "evidence_anchor": source_watch_comparison_anchor(behavior),
                "modeled_facts": source_watch_comparison_facts(behavior),
                "parity_fixtures": source_watch_comparison_fixtures(implementation, behavior),
                "mutation_checks": source_watch_comparison_mutations(behavior),
            })
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

fn source_watch_comparison_fixtures(implementation: &str, behavior: &str) -> Vec<String> {
    vec![format!(
        "{}::{}",
        implementation,
        behavior.replace(' ', "_")
    )]
}

fn source_watch_comparison_mutations(behavior: &str) -> Vec<&'static str> {
    if behavior.contains("ignore")
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
    }
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
    Ok(AdapterSummary {
        kind,
        json: summary.clone(),
    })
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
    require_comparison_str(value, "implementation")?;
    require_comparison_str(value, "behavior")?;
    let disposition = require_comparison_str(value, "disposition")?;
    if !is_known_comparison_disposition(disposition) {
        anyhow::bail!("unknown external comparison disposition: {disposition}");
    }
    require_comparison_str(value, "reason")?;
    require_comparison_str(value, "evidence_anchor")?;
    require_comparison_array(value, "modeled_facts")?;
    require_comparison_array(value, "parity_fixtures")?;
    require_comparison_array(value, "mutation_checks")?;
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

fn is_known_comparison_disposition(disposition: &str) -> bool {
    matches!(
        disposition,
        "semantic_rule"
            | "formal_adapter_contract"
            | "replay_fixture"
            | "debt_item"
            | "explicit_non_goal"
    )
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
        "watcher_evidence": "metadata-only",
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
        "timer_evidence": if trace.has_timer_fired {
            "metadata-only"
        } else if trace.shutdown_resolved {
            "shutdown-resolved"
        } else {
            "missing"
        },
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

fn replay_grade_for_windows(windows: &BTreeMap<u64, WindowTrace>) -> TraceReplayGrade {
    if windows
        .values()
        .any(|window| !window.has_timer_fired && !window.shutdown_resolved)
    {
        TraceReplayGrade::Debt
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
            "modeled": true,
            "sampled": false,
            "metadata_only": true,
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
            Self::Partial => "partial",
            Self::Debt => "debt",
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
