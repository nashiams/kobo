use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::Value;

use super::DoctorOutputFormat;

struct SupervisorSliceReport {
    status: SupervisorSliceStatus,
    state_path: PathBuf,
    surfaces: SupervisorSliceSurfaces,
    blockers: Vec<String>,
}

struct SupervisorSliceSurfaces {
    watcher_events: usize,
    debounce_windows: usize,
    restart_decisions: usize,
    child_lifecycle: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SupervisorSliceStatus {
    Ready,
    Blocked,
}

pub(super) fn cmd_supervisor_slice(
    root: &Path,
    output_format: DoctorOutputFormat,
    require_ready: bool,
    state_path: Option<&Path>,
) -> anyhow::Result<()> {
    let report = SupervisorSliceReport::from_project(root, state_path);
    emit_supervisor_slice_report(&report, output_format)?;
    if require_ready && report.status != SupervisorSliceStatus::Ready {
        anyhow::bail!("doctor supervisor slice gate blocked");
    }
    Ok(())
}

impl SupervisorSliceReport {
    fn from_project(root: &Path, state_path: Option<&Path>) -> Self {
        let state_path = state_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.join(".kobo").join("watch").join("source-watch.json"));
        let mut blockers = Vec::new();
        let mut surfaces = SupervisorSliceSurfaces {
            watcher_events: 0,
            debounce_windows: 0,
            restart_decisions: 0,
            child_lifecycle: 0,
        };

        match read_json(&state_path) {
            Ok(state) => validate_source_watch_state(&state, &mut surfaces, &mut blockers),
            Err(error) => blockers.push(format!(
                "missing or invalid supervisor slice watch evidence: {error}"
            )),
        }

        let status = if blockers.is_empty() {
            SupervisorSliceStatus::Ready
        } else {
            SupervisorSliceStatus::Blocked
        };

        Self {
            status,
            state_path,
            surfaces,
            blockers,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "schema_version": 1,
            "command": "doctor --supervisor-slice",
            "supervisor_slice": {
                "status": self.status.as_str(),
                "state_path": self.state_path.display().to_string(),
                "scope": "kobo-owned watcher/supervisor slice",
                "surfaces": {
                    "watcher_events": self.surfaces.watcher_events,
                    "debounce_windows": self.surfaces.debounce_windows,
                    "restart_decisions": self.surfaces.restart_decisions,
                    "child_lifecycle": self.surfaces.child_lifecycle,
                },
                "blockers": &self.blockers,
            },
        })
    }
}

impl SupervisorSliceStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
        }
    }
}

fn validate_source_watch_state(
    state: &Value,
    surfaces: &mut SupervisorSliceSurfaces,
    blockers: &mut Vec<String>,
) {
    if state["mode"].as_str() != Some("source_watch_state") {
        blockers.push("watch evidence mode must be source_watch_state".to_owned());
    }
    validate_watcher_evidence(state, surfaces, blockers);
    validate_debounce_windows(state, surfaces, blockers);
    validate_restart_decisions(state, surfaces, blockers);
    validate_child_lifecycle(state, surfaces, blockers);
}

fn validate_watcher_evidence(
    state: &Value,
    surfaces: &mut SupervisorSliceSurfaces,
    blockers: &mut Vec<String>,
) {
    let watcher_evidence = state["watcher_evidence"].as_str().unwrap_or("missing");
    if !is_ready_grade(watcher_evidence) {
        blockers.push(format!("watcher evidence is {watcher_evidence}"));
    }
    let Some(batches) = state["event_batches"].as_array() else {
        blockers.push("missing watcher event batches".to_owned());
        return;
    };
    if batches.is_empty() {
        blockers.push("empty watcher event batches".to_owned());
    }
    for batch in batches {
        let replay_grade = batch["replay_grade"].as_str().unwrap_or("missing");
        if !is_ready_grade(replay_grade) {
            blockers.push(format!("watcher batch replay grade is {replay_grade}"));
        }
        let Some(events) = batch["events"].as_array() else {
            blockers.push("watcher batch missing events".to_owned());
            continue;
        };
        surfaces.watcher_events += events.len();
        for event in events {
            let evidence_grade = event["evidence_grade"].as_str().unwrap_or("missing");
            if !is_ready_grade(evidence_grade) {
                blockers.push(format!("watcher event evidence grade is {evidence_grade}"));
            }
            if event["paths"].as_array().map_or(true, Vec::is_empty) {
                blockers.push("watcher event missing path evidence".to_owned());
            }
        }
    }
    if surfaces.watcher_events == 0 {
        blockers.push("missing watcher event evidence".to_owned());
    }
}

fn validate_debounce_windows(
    state: &Value,
    surfaces: &mut SupervisorSliceSurfaces,
    blockers: &mut Vec<String>,
) {
    let Some(windows) = state["debounce_windows"].as_array() else {
        blockers.push("missing debounce windows".to_owned());
        return;
    };
    surfaces.debounce_windows = windows.len();
    if windows.is_empty() {
        blockers.push("empty debounce windows".to_owned());
    }
    for window in windows {
        let timer_evidence = window["timer_evidence"].as_str().unwrap_or("missing");
        let replay_grade = window["replay_grade"].as_str().unwrap_or("missing");
        if !is_ready_grade(timer_evidence) || !is_ready_grade(replay_grade) {
            blockers.push(format!(
                "incomplete debounce evidence: timer={timer_evidence}, replay={replay_grade}"
            ));
        }
        if window["event_batch_ids"]
            .as_array()
            .map_or(true, Vec::is_empty)
        {
            blockers.push("debounce window missing event membership".to_owned());
        }
    }
}

fn validate_restart_decisions(
    state: &Value,
    surfaces: &mut SupervisorSliceSurfaces,
    blockers: &mut Vec<String>,
) {
    let Some(decisions) = state["restart_decisions"].as_array() else {
        blockers.push("missing restart decisions".to_owned());
        return;
    };
    surfaces.restart_decisions = decisions.len();
    if decisions.is_empty() {
        blockers.push("empty restart decisions".to_owned());
    }
    for decision in decisions {
        if decision["policy_branch"]
            .as_str()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            blockers.push("restart decision missing policy branch".to_owned());
        }
        if decision["action"].as_str().unwrap_or("").trim().is_empty() {
            blockers.push("restart decision missing action".to_owned());
        }
        if decision["selected_by"]
            .as_array()
            .map_or(true, Vec::is_empty)
        {
            blockers.push("restart decision missing selected paths".to_owned());
        }
    }
}

fn validate_child_lifecycle(
    state: &Value,
    surfaces: &mut SupervisorSliceSurfaces,
    blockers: &mut Vec<String>,
) {
    let Some(lifecycle) = state["child_lifecycle_obligations"].as_array() else {
        blockers.push("missing child lifecycle obligations".to_owned());
        return;
    };
    surfaces.child_lifecycle = lifecycle.len();
    if lifecycle.is_empty() {
        blockers.push("empty child lifecycle obligations".to_owned());
    }
    for entry in lifecycle {
        let resolution = entry["resolution"].as_str().unwrap_or("missing");
        if resolution.contains("unresolved") || resolution == "missing" {
            blockers.push(format!(
                "unresolved child lifecycle obligation: {resolution}"
            ));
        }
        let evidence_grade = entry["evidence_grade"].as_str().unwrap_or("missing");
        if !is_ready_grade(evidence_grade) {
            blockers.push(format!(
                "child lifecycle evidence grade is {evidence_grade}"
            ));
        }
    }
}

fn is_ready_grade(value: &str) -> bool {
    matches!(value, "exact" | "modeled")
}

fn emit_supervisor_slice_report(
    report: &SupervisorSliceReport,
    output_format: DoctorOutputFormat,
) -> anyhow::Result<()> {
    match output_format {
        DoctorOutputFormat::Json => {
            let mut value = report.to_json();
            if report.status != SupervisorSliceStatus::Ready {
                value["message"] = Value::from("doctor supervisor slice gate blocked");
            }
            println!("{}", serde_json::to_string(&value)?);
        }
        DoctorOutputFormat::Human => {
            println!("doctor --supervisor-slice: {}", report.status.as_str());
            println!("scope: kobo-owned watcher/supervisor slice");
            println!(
                "surfaces: watcher_events={}, debounce_windows={}, restart_decisions={}, child_lifecycle={}",
                report.surfaces.watcher_events,
                report.surfaces.debounce_windows,
                report.surfaces.restart_decisions,
                report.surfaces.child_lifecycle
            );
            if report.blockers.is_empty() {
                println!("blockers: none");
            } else {
                println!("blockers:");
                for blocker in &report.blockers {
                    println!("  - {blocker}");
                }
            }
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse {}", path.display()))
}
