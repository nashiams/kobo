use serde_json::Value;

const WATCHER_EVIDENCE: &str = "metadata-only";
const REPLAY_GRADE: &str = "partial";
const WATCH_BACKEND: &str = "mtime-poll";
const ORDERING_GUARANTEE: &str = "poll-order";
const DEBOUNCE_INTERVAL_MS: u64 = 200;

pub(super) enum WatchExecutionMode {
    Plan,
    Simple,
    Build,
}

pub(super) struct WatchEvidence {
    batch_id: String,
    window_id: String,
    event_kind: WatchEventKind,
    root_path: String,
    changed_path: String,
    mode: WatchExecutionMode,
    previous_modified_ms: Option<u128>,
    current_modified_ms: Option<u128>,
}

enum WatchEventKind {
    Modify,
    Rescan,
}

impl WatchEvidence {
    pub(super) fn for_plan(root_path: String, changed_path: Option<String>) -> Self {
        let event_kind = if changed_path.is_some() {
            WatchEventKind::Modify
        } else {
            WatchEventKind::Rescan
        };
        Self::new(
            1,
            event_kind,
            root_path.clone(),
            changed_path.unwrap_or(root_path),
            WatchExecutionMode::Plan,
            None,
            None,
        )
    }

    pub(super) fn for_change(
        sequence: usize,
        root_path: String,
        changed_path: String,
        mode: WatchExecutionMode,
        previous_modified_ms: u128,
        current_modified_ms: u128,
    ) -> Self {
        Self::new(
            sequence,
            WatchEventKind::Modify,
            root_path,
            changed_path,
            mode,
            Some(previous_modified_ms),
            Some(current_modified_ms),
        )
    }

    fn new(
        sequence: usize,
        event_kind: WatchEventKind,
        root_path: String,
        changed_path: String,
        mode: WatchExecutionMode,
        previous_modified_ms: Option<u128>,
        current_modified_ms: Option<u128>,
    ) -> Self {
        Self {
            batch_id: format!("watch-batch-{sequence}"),
            window_id: format!("watch-window-{sequence}"),
            event_kind,
            root_path,
            changed_path,
            mode,
            previous_modified_ms,
            current_modified_ms,
        }
    }

    pub(super) fn watcher_evidence(&self) -> &'static str {
        WATCHER_EVIDENCE
    }

    pub(super) fn batch_id(&self) -> &str {
        &self.batch_id
    }

    pub(super) fn replay_grade(&self) -> &'static str {
        REPLAY_GRADE
    }

    pub(super) fn event_kind(&self) -> &'static str {
        self.event_kind.as_str()
    }

    pub(super) fn human_lines(&self) -> Vec<String> {
        vec![
            format!("watcher evidence: {}", self.watcher_evidence()),
            format!("event batch: {}", self.batch_id),
            format!(
                "debounce window: {} ({}ms)",
                self.window_id, DEBOUNCE_INTERVAL_MS
            ),
            "restart decision: rerun".to_owned(),
            "child lifecycle: no child process started".to_owned(),
        ]
    }

    pub(super) fn event_batch_json(&self) -> Value {
        serde_json::json!({
            "id": self.batch_id,
            "backend": WATCH_BACKEND,
            "watcher_evidence": WATCHER_EVIDENCE,
            "ordering_guarantee": ORDERING_GUARANTEE,
            "replay_grade": REPLAY_GRADE,
            "events": [
                {
                    "kind": self.event_kind.as_str(),
                    "paths": [
                        {
                            "role": "source_path",
                            "path": self.changed_path,
                        }
                    ],
                    "duplicate_or_coalesced": "unknown",
                    "previous_modified_ms": self.previous_modified_ms,
                    "current_modified_ms": self.current_modified_ms,
                }
            ],
        })
    }

    pub(super) fn debounce_window_json(&self) -> Value {
        serde_json::json!({
            "id": self.window_id,
            "interval_ms": DEBOUNCE_INTERVAL_MS,
            "timer_evidence": WATCHER_EVIDENCE,
            "replay_grade": REPLAY_GRADE,
            "fired": true,
            "event_batches": [self.batch_id],
        })
    }

    pub(super) fn restart_decision_json(&self) -> Value {
        serde_json::json!({
            "action": "rerun",
            "policy_branch": "watch.changed-in-scope",
            "selected_by": self.changed_path,
            "source_scope": self.root_path,
            "rerun_targets": [
                format!("kobo check {}", self.root_path),
                format!("kobo inspect {}", self.root_path),
            ],
            "reason": "changed file belongs to scoped watch plan",
        })
    }

    pub(super) fn child_lifecycle_json(&self) -> Value {
        serde_json::json!({
            "obligation": "started child must be waited, signaled, killed, detached, or not started",
            "command_kind": self.mode.command_kind(),
            "resolution": "no_child_started",
            "reason": "watch reruns the Kobo pipeline in-process for this mode",
        })
    }
}

impl WatchEventKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Modify => "modify",
            Self::Rescan => "rescan",
        }
    }
}

impl WatchExecutionMode {
    fn command_kind(&self) -> &'static str {
        match self {
            Self::Plan => "planned_rerun",
            Self::Simple => "in_process_check",
            Self::Build => "in_process_codegen",
        }
    }
}
