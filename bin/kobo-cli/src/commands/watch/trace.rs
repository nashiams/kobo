use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{json, Value};

use crate::ErrorFormat;

const WATCH_TRACE_WITNESS_MODE: &str = "watch_trace_witness";

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum AdapterKind {
    Watcher,
    Process,
    Time,
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
struct WatcherTraceEvent {
    event_kind: String,
    path: String,
    window: u64,
    evidence_grade: String,
    duplicate_marker: String,
    raw: Value,
}

#[derive(Clone)]
struct ChildState {
    child_id: String,
    policy: String,
    command: String,
}

struct WindowTrace {
    watcher_events: Vec<WatcherTraceEvent>,
    has_timer_fired: bool,
}

struct NormalizedTrace {
    replay_grade: TraceReplayGrade,
    json: Value,
}

pub(super) fn cmd_import_trace(
    trace_path: &Path,
    witness_out: Option<&Path>,
) -> anyhow::Result<()> {
    let trace = read_trace(trace_path)?;
    let adapter_summaries = parse_adapter_summaries(&trace)?;
    let raw_events = raw_events(&trace)?;
    let normalized = normalize_trace(&raw_events)?;
    let raw_event_hash = stable_values_hash(&raw_events)?;
    let normalized_hash = stable_value_hash(&normalized.json)?;
    let adapter_summary_hash = stable_values_hash(&adapter_summary_json(&adapter_summaries))?;
    let witness = witness_json(
        trace_path,
        &adapter_summaries,
        raw_events,
        &normalized,
        &raw_event_hash,
        &normalized_hash,
        &adapter_summary_hash,
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
    let raw_events = raw_events(witness)?;
    let normalized = normalize_trace(&raw_events)?;
    let expected = json!({
        "raw_event_hash": stable_values_hash(&raw_events)?,
        "normalized_hash": stable_value_hash(&normalized.json)?,
        "adapter_summary_hash": stable_values_hash(&adapter_summary_json(&adapter_summaries))?,
        "normalized": normalized.json,
    });
    let observed = json!({
        "raw_event_hash": witness["raw_event_hash"].clone(),
        "normalized_hash": witness["normalized_hash"].clone(),
        "adapter_summary_hash": witness["adapter_summary_hash"].clone(),
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
            | TraceEventKind::ChildDetach => {
                record_child_resolution(&mut active_children, &mut child_lifecycle, event)?
            }
        }
    }

    reject_orphan_children(&active_children)?;
    let replay_grade = replay_grade_for_windows(&windows);
    Ok(NormalizedTrace {
        replay_grade,
        json: normalized_json(windows, restart_decisions, child_lifecycle, replay_grade),
    })
}

fn record_watcher_event(
    windows: &mut BTreeMap<u64, WindowTrace>,
    event: &Value,
) -> anyhow::Result<()> {
    let trace_event = WatcherTraceEvent {
        event_kind: required_str(event, "event_kind")?.to_owned(),
        path: required_str(event, "path")?.to_owned(),
        window: required_u64(event, "window")?,
        evidence_grade: event["evidence_grade"]
            .as_str()
            .unwrap_or("metadata_only")
            .to_owned(),
        duplicate_marker: event["duplicate_marker"]
            .as_str()
            .unwrap_or("unique")
            .to_owned(),
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
    Ok(())
}

fn record_child_start(
    active_children: &mut BTreeMap<String, ChildState>,
    _child_lifecycle: &mut Vec<Value>,
    event: &Value,
) -> anyhow::Result<()> {
    let child_id = required_str(event, "child_id")?.to_owned();
    let policy = event["policy"].as_str().unwrap_or("exclusive").to_owned();
    if policy == "exclusive"
        && active_children
            .values()
            .any(|child| child.policy == "exclusive")
    {
        anyhow::bail!("double-running child: exclusive restart started `{child_id}` before the active child resolved");
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
        },
    );
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
    child_lifecycle.push(json!({
        "child_id": started_child.child_id,
        "policy": started_child.policy,
        "command": started_child.command,
        "obligation": "started child must be waited, signaled, killed, or detached",
        "resolution": resolution_label(event)?,
        "exit_code": event["exit_code"].clone(),
        "source_event": event,
    }));
    Ok(())
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
        "reason": "watch trace restart policy branch selected this action",
        "source_event": event,
    }))
}

fn normalized_json(
    windows: BTreeMap<u64, WindowTrace>,
    restart_decisions: Vec<Value>,
    child_lifecycle: Vec<Value>,
    replay_grade: TraceReplayGrade,
) -> Value {
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
        "timer_evidence": if trace.has_timer_fired { "metadata-only" } else { "missing" },
        "replay_grade": if trace.has_timer_fired { replay_grade.as_str() } else { "debt" },
        "timer_created": true,
        "timer_cancelled": trace.watcher_events.len() > 1,
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
        "paths": [{
            "role": "source_path",
            "path": path,
        }],
        "duplicate_or_coalesced": if has_duplicate { "coalesced" } else { "unique" },
        "evidence_grade": evidence_grade,
        "raw_events": events.into_iter().map(|event| event.raw.clone()).collect::<Vec<_>>(),
    })
}

fn replay_grade_for_windows(windows: &BTreeMap<u64, WindowTrace>) -> TraceReplayGrade {
    if windows.values().any(|window| !window.has_timer_fired) {
        TraceReplayGrade::Debt
    } else {
        TraceReplayGrade::Partial
    }
}

fn witness_json(
    trace_path: &Path,
    adapter_summaries: &[AdapterSummary],
    raw_events: Vec<Value>,
    normalized: &NormalizedTrace,
    raw_event_hash: &str,
    normalized_hash: &str,
    adapter_summary_hash: &str,
) -> Value {
    json!({
        "schema_version": 1,
        "mode": WATCH_TRACE_WITNESS_MODE,
        "source": {
            "kind": "watch_trace",
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

fn resolution_label(event: &Value) -> anyhow::Result<&'static str> {
    match TraceEventKind::from_value(event)? {
        TraceEventKind::ChildExit => Ok("exited"),
        TraceEventKind::ChildSignal => Ok("signaled"),
        TraceEventKind::ChildKill => Ok("killed"),
        TraceEventKind::ChildDetach => Ok("detached"),
        _ => anyhow::bail!("event is not a child lifecycle resolution"),
    }
}

impl AdapterKind {
    fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "watcher" => Ok(Self::Watcher),
            "process" => Ok(Self::Process),
            "time" => Ok(Self::Time),
            _ => anyhow::bail!("unknown adapter summary kind: {value}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Watcher => "watcher",
            Self::Process => "process",
            Self::Time => "time",
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
            has_timer_fired: false,
        }
    }
}
