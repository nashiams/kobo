pub(crate) struct ModelEventExpectation {
    pub(crate) span: (usize, usize),
}

#[derive(Clone)]
pub(crate) struct ModelObligationExpectation {
    pub(crate) binding: String,
    pub(crate) span: (usize, usize),
}

#[derive(Clone)]
pub(crate) struct WardModelStep {
    pub(crate) kind: WardModelStepKind,
    pub(crate) span: (usize, usize),
}

#[derive(Clone)]
pub(crate) enum WardModelStepKind {
    EmitEvent(String),
    SetObligation {
        binding: String,
        state: String,
    },
    SetState {
        name: String,
        value: String,
    },
    SchedulerAssumption {
        preset: String,
    },
    Transition {
        name: String,
        boundary: ModelBoundaryCall,
    },
    BoundaryCall(ModelBoundaryCall),
}

#[derive(Clone, Copy)]
pub(crate) enum ModelBoundaryCall {
    WardTime,
    WardRandom,
    WardTask,
    WardTaskLocal,
    WardStorage,
    WardNetwork,
}

pub(crate) struct ModelComparisonSpec {
    pub(crate) steps: Vec<WardModelStep>,
    pub(crate) events: Vec<ModelEventExpectation>,
    pub(crate) obligations: Vec<ModelObligationExpectation>,
    pub(crate) source: &'static str,
}

pub(crate) struct WardModelRun {
    pub(crate) source: &'static str,
    pub(crate) engine: &'static str,
    pub(crate) seed: u64,
    pub(crate) ir: Vec<WardModelStep>,
    pub(crate) events: Vec<String>,
    pub(crate) states: Vec<(String, String)>,
    pub(crate) obligations: Vec<(String, String)>,
    pub(crate) scheduler_preset: Option<String>,
    pub(crate) steps_executed: usize,
}

pub(crate) struct ModelComparisonFailure {
    pub(crate) kind: &'static str,
    pub(crate) label: String,
    pub(crate) message: String,
    pub(crate) span: (usize, usize),
}

impl ModelComparisonSpec {
    pub(crate) fn requested(&self) -> bool {
        !self.steps.is_empty()
    }

    pub(crate) fn event_span(&self, index: usize) -> Option<(usize, usize)> {
        self.steps
            .iter()
            .filter(|step| step.kind.emits_event())
            .nth(index)
            .map(|step| step.span)
    }

    pub(crate) fn obligation_span(&self, binding: &str) -> Option<(usize, usize)> {
        self.steps.iter().find_map(|step| match &step.kind {
            WardModelStepKind::SetObligation {
                binding: candidate, ..
            } if candidate == binding => Some(step.span),
            _ => None,
        })
    }
}

impl WardModelStepKind {
    pub(crate) fn emits_event(&self) -> bool {
        matches!(
            self,
            Self::EmitEvent(_)
                | Self::Transition { .. }
                | Self::BoundaryCall(ModelBoundaryCall::WardTime)
                | Self::BoundaryCall(ModelBoundaryCall::WardRandom)
                | Self::BoundaryCall(ModelBoundaryCall::WardTask)
                | Self::BoundaryCall(ModelBoundaryCall::WardTaskLocal)
                | Self::BoundaryCall(ModelBoundaryCall::WardStorage)
                | Self::BoundaryCall(ModelBoundaryCall::WardNetwork)
        )
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        match self {
            Self::EmitEvent(event) => serde_json::json!({
                "kind": "emit_event",
                "event": event,
            }),
            Self::SetObligation { binding, state } => serde_json::json!({
                "kind": "set_obligation",
                "binding": binding,
                "state": state,
            }),
            Self::SetState { name, value } => serde_json::json!({
                "kind": "set_state",
                "name": name,
                "value": value,
            }),
            Self::SchedulerAssumption { preset } => serde_json::json!({
                "kind": "scheduler_assumption",
                "preset": preset,
            }),
            Self::Transition { name, boundary } => serde_json::json!({
                "kind": "transition",
                "name": name,
                "target": boundary.as_str(),
                "emits": boundary.event_kind(),
            }),
            Self::BoundaryCall(boundary) => serde_json::json!({
                "kind": "boundary_call",
                "target": boundary.as_str(),
                "emits": boundary.event_kind(),
            }),
        }
    }
}

impl ModelBoundaryCall {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
            Self::WardTaskLocal => "ward.task.local",
            Self::WardStorage => "ward.storage",
            Self::WardNetwork => "ward.network",
        }
    }

    pub(crate) fn event_kind(self) -> &'static str {
        match self {
            Self::WardTime => "deterministic-time",
            Self::WardRandom => "deterministic-random",
            Self::WardTask | Self::WardTaskLocal => "deterministic-task",
            Self::WardStorage => "storage-boundary",
            Self::WardNetwork => "network-boundary",
        }
    }
}
