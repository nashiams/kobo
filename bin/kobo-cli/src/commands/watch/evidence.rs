use serde_json::Value;

pub(super) const DEBOUNCE_INTERVAL_MS: u64 = 200;

#[derive(Clone)]
pub(super) struct WatchEventInput {
    pub paths: Vec<WatchEventPathInput>,
    pub event_kind: WatchEventKind,
    pub previous_modified_ms: Option<u128>,
    pub current_modified_ms: Option<u128>,
    pub duplicate_status: DuplicateStatus,
    pub evidence_grade: EventEvidenceGrade,
    pub raw_events: Vec<RawWatchEventInput>,
}

#[derive(Clone)]
pub(super) struct WatchEventPathInput {
    pub role: WatchPathRole,
    pub path: String,
}

#[derive(Clone)]
pub(super) struct RawWatchEventInput {
    pub event_kind: WatchEventKind,
    pub paths: Vec<WatchEventPathInput>,
}

pub(super) struct DebounceWindowInput {
    pub has_timer_extension: bool,
    pub extension_cause_paths: Vec<String>,
}

#[derive(Clone, Copy)]
pub(super) enum WatchExecutionMode {
    Plan,
    Simple,
    Build,
}

#[derive(Clone, Copy)]
pub(super) enum WatchEventKind {
    Create,
    Modify,
    Remove,
    RenameCandidate,
    Metadata,
    Unknown,
    Rescan,
}

#[derive(Clone, Copy)]
pub(super) enum DuplicateStatus {
    Unique,
    Coalesced,
    Unknown,
}

#[derive(Clone, Copy)]
pub(super) enum EventEvidenceGrade {
    MetadataOnly,
    ModelledFromMetadata,
    Unknown,
}

#[derive(Clone, Copy)]
pub(super) enum RestartPolicyBranch {
    ChangedInScope,
    IgnoredOutOfScope,
    PlanOnly,
}

#[derive(Clone, Copy)]
pub(super) enum WatchRerunOutcome {
    Planned,
    Succeeded,
    Failed,
}

pub(super) struct WatchRerunReport {
    pub outcome: WatchRerunOutcome,
    pub diagnostic_count: usize,
}

pub(super) struct WatchEvidence {
    batch: WatchEventBatch,
    debounce_window: DebounceWindowEvidence,
    restart_decision: RestartDecisionEvidence,
    child_lifecycle: ChildLifecycleEvidence,
}

struct WatchEventBatch {
    id: WatchBatchId,
    backend: WatchBackend,
    watcher_evidence: EvidenceGrade,
    ordering_guarantee: OrderingGuarantee,
    replay_grade: ReplayGrade,
    events: Vec<WatchEventEvidence>,
}

struct WatchEventEvidence {
    event_kind: WatchEventKind,
    paths: Vec<WatchEventPath>,
    duplicate_status: DuplicateStatus,
    previous_modified_ms: Option<u128>,
    current_modified_ms: Option<u128>,
    evidence_grade: EventEvidenceGrade,
    raw_events: Vec<RawWatchEventEvidence>,
}

struct WatchEventPath {
    role: WatchPathRole,
    path: String,
}

struct RawWatchEventEvidence {
    event_kind: WatchEventKind,
    paths: Vec<WatchEventPath>,
}

struct DebounceWindowEvidence {
    id: WatchWindowId,
    interval_ms: u64,
    timer_evidence: EvidenceGrade,
    replay_grade: ReplayGrade,
    has_timer_creation: bool,
    has_timer_cancellation: bool,
    has_fired: bool,
    extension_cause_paths: Vec<String>,
    event_memberships: Vec<DebounceEventMembership>,
    event_batches: Vec<WatchBatchId>,
}

struct DebounceEventMembership {
    batch_id: WatchBatchId,
    path: String,
}

struct RestartDecisionEvidence {
    action: RestartAction,
    policy_branch: RestartPolicyBranch,
    selected_paths: Vec<String>,
    source_scope: String,
    rerun_targets: Vec<String>,
    reason: RestartReason,
    outcome: WatchRerunOutcome,
    diagnostic_count: usize,
}

struct ChildLifecycleEvidence {
    obligation: ChildLifecycleObligation,
    command_kind: WatchCommandKind,
    resolution: ChildLifecycleResolution,
    reason: ChildLifecycleReason,
}

#[derive(Clone)]
struct WatchBatchId(String);

#[derive(Clone)]
struct WatchWindowId(String);

#[derive(Clone, Copy)]
enum WatchBackend {
    MtimePoll,
}

#[derive(Clone, Copy)]
enum EvidenceGrade {
    MetadataOnly,
}

#[derive(Clone, Copy)]
enum OrderingGuarantee {
    PollOrder,
}

#[derive(Clone, Copy)]
enum ReplayGrade {
    Partial,
}

#[derive(Clone, Copy)]
pub(super) enum WatchPathRole {
    SourcePath,
    DestinationPath,
    ParentPath,
}

#[derive(Clone, Copy)]
enum RestartAction {
    Rerun,
    NoOp,
}

#[derive(Clone, Copy)]
enum RestartReason {
    ChangedFileInScope,
    ChangedFileOutOfScope,
    PlanOnly,
}

#[derive(Clone, Copy)]
enum WatchCommandKind {
    PlannedRerun,
    InProcessCheck,
    InProcessCodegen,
}

#[derive(Clone, Copy)]
enum ChildLifecycleObligation {
    ResolveStartedChild,
}

#[derive(Clone, Copy)]
enum ChildLifecycleResolution {
    NoChildStarted,
    InProcessRerunFinished,
}

#[derive(Clone, Copy)]
enum ChildLifecycleReason {
    PlanDoesNotStartChild,
    PipelineRunsInProcess,
}

impl WatchEvidence {
    pub(super) fn for_plan(
        sequence: usize,
        root_path: String,
        changed_path: Option<String>,
        branch: RestartPolicyBranch,
    ) -> Self {
        let selected_paths = changed_path
            .clone()
            .map(|path| vec![path])
            .unwrap_or_default();
        let event_kind = if selected_paths.is_empty() {
            WatchEventKind::Rescan
        } else {
            WatchEventKind::Modify
        };
        Self::new(
            sequence,
            root_path,
            selected_paths,
            vec![WatchEventInput {
                paths: vec![WatchEventPathInput {
                    role: WatchPathRole::SourcePath,
                    path: changed_path.unwrap_or_else(|| "planned scope".to_owned()),
                }],
                event_kind,
                previous_modified_ms: None,
                current_modified_ms: None,
                duplicate_status: DuplicateStatus::Unknown,
                evidence_grade: EventEvidenceGrade::Unknown,
                raw_events: Vec::new(),
            }],
            DebounceWindowInput {
                has_timer_extension: false,
                extension_cause_paths: Vec::new(),
            },
            WatchExecutionMode::Plan,
            branch,
            WatchRerunOutcome::Planned,
            0,
        )
    }

    pub(super) fn for_window(
        sequence: usize,
        root_path: String,
        events: Vec<WatchEventInput>,
        debounce: DebounceWindowInput,
        mode: WatchExecutionMode,
    ) -> Self {
        let selected_paths = events
            .iter()
            .filter_map(WatchEventInput::primary_path)
            .map(str::to_owned)
            .collect();
        Self::new(
            sequence,
            root_path,
            selected_paths,
            events,
            debounce,
            mode,
            RestartPolicyBranch::ChangedInScope,
            WatchRerunOutcome::Planned,
            0,
        )
    }

    fn new(
        sequence: usize,
        root_path: String,
        selected_paths: Vec<String>,
        events: Vec<WatchEventInput>,
        debounce: DebounceWindowInput,
        mode: WatchExecutionMode,
        branch: RestartPolicyBranch,
        outcome: WatchRerunOutcome,
        diagnostic_count: usize,
    ) -> Self {
        let batch_id = WatchBatchId::new(sequence);
        let window_id = WatchWindowId::new(sequence);
        let event_batch = WatchEventBatch::new(batch_id.clone(), events);
        let debounce_window =
            DebounceWindowEvidence::new(window_id, batch_id, &event_batch, debounce);
        let restart_decision = RestartDecisionEvidence::new(
            branch,
            selected_paths,
            root_path,
            outcome,
            diagnostic_count,
        );
        let child_lifecycle = ChildLifecycleEvidence::new(mode, branch, outcome);
        Self {
            batch: event_batch,
            debounce_window,
            restart_decision,
            child_lifecycle,
        }
    }

    pub(super) fn record_rerun_report(&mut self, report: WatchRerunReport) {
        self.restart_decision.outcome = report.outcome;
        self.restart_decision.diagnostic_count = report.diagnostic_count;
        self.child_lifecycle.resolution = ChildLifecycleResolution::InProcessRerunFinished;
        self.child_lifecycle.reason = ChildLifecycleReason::PipelineRunsInProcess;
    }

    pub(super) fn watcher_evidence(&self) -> &'static str {
        self.batch.watcher_evidence.as_str()
    }

    pub(super) fn changed_paths(&self) -> Vec<&str> {
        self.restart_decision
            .selected_paths
            .iter()
            .map(String::as_str)
            .collect()
    }

    pub(super) fn human_lines(&self) -> Vec<String> {
        vec![
            format!("watcher evidence: {}", self.watcher_evidence()),
            format!("event batch: {}", self.batch.id.as_str()),
            format!(
                "debounce window: {} ({}ms)",
                self.debounce_window.id.as_str(),
                self.debounce_window.interval_ms
            ),
            format!(
                "restart decision: {}",
                self.restart_decision.action.as_str()
            ),
            format!(
                "child lifecycle: {}",
                self.child_lifecycle.resolution.human_label()
            ),
        ]
    }

    pub(super) fn changes_json(&self) -> Vec<Value> {
        self.batch
            .events
            .iter()
            .map(|event| {
                serde_json::json!({
                    "kind": "source_change",
                    "path": event.primary_path(),
                    "event_kind": event.event_kind.as_str(),
                    "batch_id": self.batch.id.as_str(),
                    "replay_grade": self.batch.replay_grade.as_str(),
                    "evidence_grade": event.evidence_grade.as_str(),
                    "previous_modified_ms": event.previous_modified_ms,
                    "current_modified_ms": event.current_modified_ms,
                    "raw_events": event.raw_events.iter().map(RawWatchEventEvidence::to_json).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    pub(super) fn event_batch_json(&self) -> Value {
        self.batch.to_json()
    }

    pub(super) fn debounce_window_json(&self) -> Value {
        self.debounce_window.to_json()
    }

    pub(super) fn restart_decision_json(&self) -> Value {
        self.restart_decision.to_json()
    }

    pub(super) fn child_lifecycle_json(&self) -> Value {
        self.child_lifecycle.to_json()
    }
}

impl WatchEventBatch {
    fn new(id: WatchBatchId, events: Vec<WatchEventInput>) -> Self {
        Self {
            id,
            backend: WatchBackend::MtimePoll,
            watcher_evidence: EvidenceGrade::MetadataOnly,
            ordering_guarantee: OrderingGuarantee::PollOrder,
            replay_grade: ReplayGrade::Partial,
            events: events.into_iter().map(WatchEventEvidence::from).collect(),
        }
    }

    fn to_json(&self) -> Value {
        let known_event_kinds = [
            WatchEventKind::Create,
            WatchEventKind::Modify,
            WatchEventKind::Remove,
            WatchEventKind::RenameCandidate,
            WatchEventKind::Metadata,
            WatchEventKind::Unknown,
            WatchEventKind::Rescan,
        ]
        .into_iter()
        .map(|event_kind| event_kind.as_str())
        .collect::<Vec<_>>();
        let known_path_roles = [
            WatchPathRole::SourcePath,
            WatchPathRole::DestinationPath,
            WatchPathRole::ParentPath,
        ]
        .into_iter()
        .map(|path_role| path_role.as_str())
        .collect::<Vec<_>>();
        serde_json::json!({
            "id": self.id.as_str(),
            "backend": self.backend.as_str(),
            "watcher_evidence": self.watcher_evidence.as_str(),
            "ordering_guarantee": self.ordering_guarantee.as_str(),
            "replay_grade": self.replay_grade.as_str(),
            "known_event_kinds": known_event_kinds,
            "known_path_roles": known_path_roles,
            "events": self.events.iter().map(WatchEventEvidence::to_json).collect::<Vec<_>>(),
        })
    }
}

impl WatchEventEvidence {
    fn primary_path(&self) -> &str {
        self.paths
            .first()
            .map(|path| path.path.as_str())
            .unwrap_or("")
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "kind": self.event_kind.as_str(),
            "paths": self.paths.iter().map(WatchEventPath::to_json).collect::<Vec<_>>(),
            "duplicate_or_coalesced": self.duplicate_status.as_str(),
            "previous_modified_ms": self.previous_modified_ms,
            "current_modified_ms": self.current_modified_ms,
            "evidence_grade": self.evidence_grade.as_str(),
            "raw_events": self.raw_events.iter().map(RawWatchEventEvidence::to_json).collect::<Vec<_>>(),
        })
    }
}

impl From<WatchEventInput> for WatchEventEvidence {
    fn from(input: WatchEventInput) -> Self {
        Self {
            event_kind: input.event_kind,
            paths: input
                .paths
                .into_iter()
                .map(|path| WatchEventPath {
                    role: path.role,
                    path: path.path,
                })
                .collect(),
            duplicate_status: input.duplicate_status,
            previous_modified_ms: input.previous_modified_ms,
            current_modified_ms: input.current_modified_ms,
            evidence_grade: input.evidence_grade,
            raw_events: input
                .raw_events
                .into_iter()
                .map(RawWatchEventEvidence::from)
                .collect(),
        }
    }
}

impl WatchEventInput {
    fn primary_path(&self) -> Option<&str> {
        self.paths.first().map(|path| path.path.as_str())
    }
}

impl From<RawWatchEventInput> for RawWatchEventEvidence {
    fn from(input: RawWatchEventInput) -> Self {
        Self {
            event_kind: input.event_kind,
            paths: input
                .paths
                .into_iter()
                .map(|path| WatchEventPath {
                    role: path.role,
                    path: path.path,
                })
                .collect(),
        }
    }
}

impl WatchEventPath {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "role": self.role.as_str(),
            "path": self.path,
        })
    }
}

impl RawWatchEventEvidence {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "kind": self.event_kind.as_str(),
            "paths": self.paths.iter().map(WatchEventPath::to_json).collect::<Vec<_>>(),
        })
    }
}

impl DebounceWindowEvidence {
    fn new(
        id: WatchWindowId,
        batch_id: WatchBatchId,
        batch: &WatchEventBatch,
        debounce: DebounceWindowInput,
    ) -> Self {
        let event_memberships = batch
            .events
            .iter()
            .map(|event| DebounceEventMembership {
                batch_id: batch_id.clone(),
                path: event.primary_path().to_owned(),
            })
            .collect();
        Self {
            id,
            interval_ms: DEBOUNCE_INTERVAL_MS,
            timer_evidence: EvidenceGrade::MetadataOnly,
            replay_grade: ReplayGrade::Partial,
            has_timer_creation: true,
            has_timer_cancellation: debounce.has_timer_extension,
            has_fired: true,
            extension_cause_paths: debounce.extension_cause_paths,
            event_memberships,
            event_batches: vec![batch_id],
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id.as_str(),
            "interval_ms": self.interval_ms,
            "timer_evidence": self.timer_evidence.as_str(),
            "replay_grade": self.replay_grade.as_str(),
            "timer_created": self.has_timer_creation,
            "timer_cancelled": self.has_timer_cancellation,
            "extension_cause": self.extension_cause_paths,
            "event_membership": self.event_memberships.iter().map(DebounceEventMembership::to_json).collect::<Vec<_>>(),
            "fired": self.has_fired,
            "event_batches": self.event_batches.iter().map(WatchBatchId::as_str).collect::<Vec<_>>(),
        })
    }
}

impl DebounceEventMembership {
    fn to_json(&self) -> Value {
        serde_json::json!({
            "batch_id": self.batch_id.as_str(),
            "path": self.path,
        })
    }
}

impl RestartDecisionEvidence {
    fn new(
        policy_branch: RestartPolicyBranch,
        selected_paths: Vec<String>,
        source_scope: String,
        outcome: WatchRerunOutcome,
        diagnostic_count: usize,
    ) -> Self {
        Self {
            action: policy_branch.restart_action(),
            policy_branch,
            selected_paths,
            rerun_targets: vec![
                format!("kobo check {source_scope}"),
                format!("kobo inspect {source_scope}"),
            ],
            source_scope,
            reason: policy_branch.reason(),
            outcome,
            diagnostic_count,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "action": self.action.as_str(),
            "policy_branch": self.policy_branch.as_str(),
            "selected_by": self.selected_paths,
            "source_scope": self.source_scope,
            "rerun_targets": self.rerun_targets,
            "reason": self.reason.as_str(),
            "outcome": self.outcome.as_str(),
            "diagnostic_count": self.diagnostic_count,
        })
    }
}

impl ChildLifecycleEvidence {
    fn new(
        mode: WatchExecutionMode,
        branch: RestartPolicyBranch,
        outcome: WatchRerunOutcome,
    ) -> Self {
        let command_kind = WatchCommandKind::from(mode);
        let resolution = if branch.restart_action().is_rerun()
            && !matches!(outcome, WatchRerunOutcome::Planned)
        {
            ChildLifecycleResolution::InProcessRerunFinished
        } else {
            ChildLifecycleResolution::NoChildStarted
        };
        let reason = match resolution {
            ChildLifecycleResolution::NoChildStarted => ChildLifecycleReason::PlanDoesNotStartChild,
            ChildLifecycleResolution::InProcessRerunFinished => {
                ChildLifecycleReason::PipelineRunsInProcess
            }
        };
        Self {
            obligation: ChildLifecycleObligation::ResolveStartedChild,
            command_kind,
            resolution,
            reason,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "obligation": self.obligation.as_str(),
            "command_kind": self.command_kind.as_str(),
            "resolution": self.resolution.as_str(),
            "reason": self.reason.as_str(),
        })
    }
}

impl WatchBatchId {
    fn new(sequence: usize) -> Self {
        Self(format!("watch-batch-{sequence}"))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl WatchWindowId {
    fn new(sequence: usize) -> Self {
        Self(format!("watch-window-{sequence}"))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl WatchBackend {
    fn as_str(&self) -> &'static str {
        match self {
            Self::MtimePoll => "mtime-poll",
        }
    }
}

impl EvidenceGrade {
    fn as_str(&self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata-only",
        }
    }
}

impl OrderingGuarantee {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PollOrder => "poll-order",
        }
    }
}

impl ReplayGrade {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Partial => "partial",
        }
    }
}

impl WatchEventKind {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Modify => "modify",
            Self::Remove => "remove",
            Self::RenameCandidate => "rename_candidate",
            Self::Metadata => "metadata",
            Self::Unknown => "unknown",
            Self::Rescan => "rescan",
        }
    }
}

impl EventEvidenceGrade {
    fn as_str(&self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata_only",
            Self::ModelledFromMetadata => "modelled_from_metadata",
            Self::Unknown => "unknown",
        }
    }
}

impl WatchPathRole {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SourcePath => "source_path",
            Self::DestinationPath => "destination_path",
            Self::ParentPath => "parent_path",
        }
    }
}

impl DuplicateStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Unique => "unique",
            Self::Coalesced => "coalesced",
            Self::Unknown => "unknown",
        }
    }
}

impl RestartPolicyBranch {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ChangedInScope => "watch.changed-in-scope",
            Self::IgnoredOutOfScope => "watch.ignored-out-of-scope",
            Self::PlanOnly => "watch.plan-only",
        }
    }

    fn reason(&self) -> RestartReason {
        match self {
            Self::ChangedInScope => RestartReason::ChangedFileInScope,
            Self::IgnoredOutOfScope => RestartReason::ChangedFileOutOfScope,
            Self::PlanOnly => RestartReason::PlanOnly,
        }
    }

    fn restart_action(&self) -> RestartAction {
        match self {
            Self::ChangedInScope => RestartAction::Rerun,
            Self::IgnoredOutOfScope | Self::PlanOnly => RestartAction::NoOp,
        }
    }
}

impl RestartAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Rerun => "rerun",
            Self::NoOp => "no-op",
        }
    }

    fn is_rerun(&self) -> bool {
        matches!(self, Self::Rerun)
    }
}

impl RestartReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ChangedFileInScope => "changed file belongs to scoped watch plan",
            Self::ChangedFileOutOfScope => "changed file is outside scoped watch plan",
            Self::PlanOnly => "watch plan has no changed file",
        }
    }
}

impl WatchCommandKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PlannedRerun => "planned_rerun",
            Self::InProcessCheck => "in_process_check",
            Self::InProcessCodegen => "in_process_codegen",
        }
    }
}

impl From<WatchExecutionMode> for WatchCommandKind {
    fn from(value: WatchExecutionMode) -> Self {
        match value {
            WatchExecutionMode::Plan => Self::PlannedRerun,
            WatchExecutionMode::Simple => Self::InProcessCheck,
            WatchExecutionMode::Build => Self::InProcessCodegen,
        }
    }
}

impl WatchRerunOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

impl ChildLifecycleObligation {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ResolveStartedChild => {
                "started child must be waited, signaled, killed, detached, or not started"
            }
        }
    }
}

impl ChildLifecycleResolution {
    fn as_str(&self) -> &'static str {
        match self {
            Self::NoChildStarted => "no_child_started",
            Self::InProcessRerunFinished => "in_process_rerun_finished",
        }
    }

    fn human_label(&self) -> &'static str {
        match self {
            Self::NoChildStarted => "no child process started",
            Self::InProcessRerunFinished => "in-process rerun finished",
        }
    }
}

impl ChildLifecycleReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PlanDoesNotStartChild => "watch planning does not start a child process",
            Self::PipelineRunsInProcess => "watch reruns the Kobo pipeline in-process",
        }
    }
}
