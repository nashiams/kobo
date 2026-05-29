use kobo_errors::KErrorCode;
use kobo_ir::{ScenarioBoundaryCallArgument, ScenarioExternalCallShape, ScenarioProgram};

use crate::error::Result;
use crate::harness_manifest::HarnessManifest;
use crate::network::NetworkModel;
use crate::storage::StorageModel;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineMode {
    SemanticOnly,
    HarnessOnly,
    Both,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioOptions {
    pub sim_profile: String,
    pub profile: String,
    pub seed: u64,
    pub inject: Option<String>,
    pub event_budget: Option<u64>,
    pub scheduler: SchedulerPolicy,
    pub loom_max_branches: Option<u64>,
    pub loom_checkpoint_replay: bool,
}

impl Default for ScenarioOptions {
    fn default() -> Self {
        Self {
            sim_profile: "quick".to_owned(),
            profile: "checked".to_owned(),
            seed: 0,
            inject: None,
            event_budget: None,
            scheduler: SchedulerPolicy::Default,
            loom_max_branches: None,
            loom_checkpoint_replay: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerPolicy {
    Default,
    RoundRobin,
    Pct,
    Exhaustive,
    SmallRandom,
}

impl SchedulerPolicy {
    pub fn from_name(name: Option<&str>) -> Self {
        match name {
            Some("round-robin") => Self::RoundRobin,
            Some("pct") => Self::Pct,
            Some("exhaustive") => Self::Exhaustive,
            Some("small-random") => Self::SmallRandom,
            _ => Self::Default,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::RoundRobin => "round-robin",
            Self::Pct => "pct",
            Self::Exhaustive => "exhaustive",
            Self::SmallRandom => "small-random",
        }
    }

    pub fn effective_for_profile(&self, sim_profile: &str) -> Self {
        match self {
            Self::Default => match sim_profile {
                "deep" => Self::Pct,
                "exhaustive" => Self::Exhaustive,
                _ => Self::RoundRobin,
            },
            explicit => explicit.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FullDepthRun {
    pub target: String,
    pub profile: String,
    pub replay_guarantee: ReplayGuarantee,
    pub events: Vec<ScenarioEvent>,
    pub failure: Option<ScenarioFailure>,
    pub coverage: ScenarioCoverage,
    pub digest: ExecutionDigest,
    pub modeled_boundaries: Vec<ModeledBoundary>,
    pub opaque_boundaries: Vec<String>,
    pub obligations: Vec<RuntimeObligationSummary>,
    pub boundary_decisions: Vec<BoundaryDecision>,
    pub harness_manifest: Option<HarnessManifest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplayGuarantee {
    Exact,
    Partial,
    NotReplayable,
}

impl ReplayGuarantee {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Partial => "partial",
            Self::NotReplayable => "not_replayable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioCoverage {
    pub unsupported_constructs: Vec<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionDigest {
    pub semantic_engine: String,
    pub harness_engine: String,
    pub model_schema: String,
    pub schema_version: u64,
    pub scenario_ir_hash: String,
    pub operation_count: usize,
    pub semantic_trace_hash: String,
    pub harness_trace_hash: String,
    pub fuzz_driver_trace_hash: Option<String>,
    pub fuzz_driver_event_count: usize,
    pub agreement: String,
    pub generated_rust_hash: Option<String>,
    pub harness_manifest_hash: Option<String>,
    pub harness_exit_code: Option<i32>,
    pub harness_event_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioFailure {
    pub code: KErrorCode,
    pub message: String,
    pub primary_start: usize,
    pub primary_end: usize,
    pub events: Vec<ScenarioEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScenarioEvent {
    pub kind: String,
    pub label: Option<String>,
    pub value: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io: Option<BoundaryIoCapture>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoundaryIoCapture {
    pub mode: String,
    pub replay_key: String,
    pub request: BoundaryIoPayload,
    pub response: BoundaryIoPayload,
    pub request_hash: String,
    pub response_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoundaryIoPayload {
    pub kind: String,
    pub fields: Vec<BoundaryIoField>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BoundaryIoField {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeObligationSummary {
    pub binding: String,
    pub type_name: String,
    pub actions: Vec<String>,
    pub is_discharged: bool,
    pub declaration_span: (usize, usize),
    pub drop_span: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryDecision {
    pub crate_name: String,
    pub call_path: Option<String>,
    pub call_arguments: Vec<ScenarioBoundaryCallArgument>,
    pub return_type: Option<String>,
    pub call_shape: ScenarioExternalCallShape,
    pub policy: BoundaryPolicyChoice,
    pub reason: Option<String>,
    pub span_start: usize,
    pub span_end: usize,
    pub recorded_io: Option<BoundaryIoCapture>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryPolicyChoice {
    Typed,
    Model,
    Record,
    Activity,
    Stub,
    Outside,
    Opaque,
    Debt,
    Unselected,
}

impl BoundaryPolicyChoice {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Model => "model",
            Self::Record => "record",
            Self::Activity => "activity",
            Self::Stub => "stub",
            Self::Outside => "outside",
            Self::Opaque => "opaque",
            Self::Debt => "debt",
            Self::Unselected => "unselected",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModeledBoundary {
    WardTime,
    WardRandom,
    WardTask,
    WardTaskLocal,
    WardStorage,
    WardNetwork,
}

impl ModeledBoundary {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
            Self::WardTaskLocal => "ward.task.local",
            Self::WardStorage => "ward.storage",
            Self::WardNetwork => "ward.network",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioOperation {
    CreateObligation {
        binding: String,
        type_name: String,
        actions: Vec<String>,
        span_start: usize,
        span_end: usize,
    },
    Discharge {
        binding: String,
        action: String,
    },
    Transfer {
        binding: String,
        callee: String,
        span_start: usize,
        span_end: usize,
    },
    MoveBinding {
        binding: String,
        span_start: usize,
        span_end: usize,
    },
    BranchUnresolved {
        binding: String,
        span_start: usize,
        span_end: usize,
    },
    UnsupportedContainer {
        binding: String,
        type_name: String,
        container: String,
        span_start: usize,
        span_end: usize,
    },
    ModeledEffect {
        boundary: ModeledBoundary,
        span_start: usize,
        span_end: usize,
    },
    StorageEvent {
        action: String,
        span_start: usize,
        span_end: usize,
    },
    NetworkEvent {
        action: String,
        span_start: usize,
        span_end: usize,
    },
    Select {
        branch_count: u32,
        span_start: usize,
        span_end: usize,
    },
    RawNondeterminism {
        operation: String,
        span_start: usize,
        span_end: usize,
    },
    UncontrolledEffect {
        operation: String,
        span_start: usize,
        span_end: usize,
    },
    ExternalBoundary {
        crate_name: String,
        call_path: Option<String>,
        call_arguments: Vec<ScenarioBoundaryCallArgument>,
        return_type: Option<String>,
        call_shape: ScenarioExternalCallShape,
        policy: BoundaryPolicyChoice,
        reason: Option<String>,
        span_start: usize,
        span_end: usize,
    },
    Loop {
        span_start: usize,
        span_end: usize,
    },
}

pub fn run_compiler_semantics(source: &str, target: &str) -> Result<FullDepthRun> {
    let options = ScenarioOptions::default();
    let artifacts = crate::source::compile_source_for_replay(source, target, &options.profile)?;
    run_semantics_from_program(&artifacts.program, &options)
}

pub fn run_semantics_from_program(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Result<FullDepthRun> {
    let lowered = crate::lower::lower_from_program(program, &options.profile);
    Ok(execute_lowered(&program.target, options, lowered))
}

pub fn run_full_depth(
    source: &str,
    target: &str,
    mode: EngineMode,
    options: ScenarioOptions,
) -> Result<FullDepthRun> {
    let artifacts = crate::source::compile_source_for_replay(source, target, &options.profile)?;
    run_full_depth_from_program(
        &artifacts.program,
        &artifacts.generated_rust,
        &options,
        mode,
    )
}

pub fn run_full_depth_from_program(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    mode: EngineMode,
) -> Result<FullDepthRun> {
    match mode {
        EngineMode::SemanticOnly => {
            let mut run = run_semantics_from_program(program, options)?;
            run.replay_guarantee = ReplayGuarantee::Partial;
            run.coverage.reason =
                Some("semantic-only execution has no generated harness agreement".to_owned());
            run.digest.harness_engine = "none".to_owned();
            run.digest.harness_trace_hash.clear();
            run.digest.agreement = "semantic-only".to_owned();
            Ok(run)
        }
        EngineMode::HarnessOnly => {
            let semantic = run_semantics_from_program(program, options)?;
            crate::harness::check_harness_agreement(
                program,
                generated_rust,
                options,
                semantic,
                EngineMode::HarnessOnly,
            )
        }
        EngineMode::Both => {
            let semantic = run_semantics_from_program(program, options)?;
            crate::harness::check_harness_agreement(
                program,
                generated_rust,
                options,
                semantic,
                EngineMode::Both,
            )
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoweredScenario {
    pub profile: String,
    pub operations: Vec<ScenarioOperation>,
    pub coverage: ScenarioCoverage,
}

#[derive(Clone, Debug)]
struct RuntimeObligation {
    binding: String,
    type_name: String,
    actions: Vec<String>,
    declaration_span: (usize, usize),
    drop_span: Option<(usize, usize)>,
    is_discharged: bool,
}

struct Runtime<'a> {
    options: &'a ScenarioOptions,
    events: Vec<ScenarioEvent>,
    modeled_boundaries: Vec<ModeledBoundary>,
    opaque_boundaries: Vec<String>,
    obligations: Vec<RuntimeObligation>,
    boundary_decisions: Vec<BoundaryDecision>,
    storage: StorageModel,
    network: NetworkModel,
    failure: Option<ScenarioFailure>,
    applied_injection: bool,
}

fn execute_lowered(
    target: &str,
    options: &ScenarioOptions,
    lowered: LoweredScenario,
) -> FullDepthRun {
    let mut runtime = Runtime::new(options);
    runtime.execute(&lowered.operations);
    runtime.finish(target, lowered, "semantic-only")
}

impl<'a> Runtime<'a> {
    fn new(options: &'a ScenarioOptions) -> Self {
        Self {
            options,
            events: Vec::new(),
            modeled_boundaries: Vec::new(),
            opaque_boundaries: Vec::new(),
            obligations: Vec::new(),
            boundary_decisions: Vec::new(),
            storage: StorageModel::default(),
            network: NetworkModel::default(),
            failure: None,
            applied_injection: false,
        }
    }

    fn execute(&mut self, operations: &[ScenarioOperation]) {
        for operation in operations {
            match operation {
                ScenarioOperation::CreateObligation {
                    binding,
                    type_name,
                    actions,
                    span_start,
                    span_end,
                } => self.obligations.push(RuntimeObligation {
                    binding: binding.clone(),
                    type_name: type_name.clone(),
                    actions: actions.clone(),
                    declaration_span: (*span_start, *span_end),
                    drop_span: None,
                    is_discharged: false,
                }),
                ScenarioOperation::Discharge { binding, action } => {
                    self.storage.note_obligation_action(action);
                    self.discharge(binding, action);
                }
                ScenarioOperation::Transfer {
                    binding,
                    callee,
                    span_start,
                    span_end,
                } => self.record_transfer(binding, callee, (*span_start, *span_end)),
                ScenarioOperation::MoveBinding {
                    binding,
                    span_start,
                    span_end,
                } => {
                    if let Some(obligation) = self
                        .obligations
                        .iter_mut()
                        .rev()
                        .find(|obligation| obligation.binding == *binding)
                    {
                        obligation.drop_span = Some((*span_start, *span_end));
                    }
                }
                ScenarioOperation::BranchUnresolved {
                    binding,
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0100,
                    message: format!(
                        "strict liveness: unresolved obligation `{binding}` reaches one branch exit"
                    ),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "branch-unresolved".to_owned(),
                        label: Some(binding.clone()),
                        value: None,
                        io: None,
                    }],
                }),
                ScenarioOperation::UnsupportedContainer {
                    binding,
                    type_name,
                    container,
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0100,
                    message: format!(
                        "strict liveness: {container} containing {type_name} `{binding}` needs an obligation-aware wrapper or declaration"
                    ),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "unsupported-container".to_owned(),
                        label: Some(format!("{binding}:{container}:{type_name}")),
                        value: None,
                        io: None,
                    }],
                }),
                ScenarioOperation::ModeledEffect {
                    boundary,
                    span_start,
                    span_end,
                } => self.record_modeled_effect(boundary.clone(), (*span_start, *span_end)),
                ScenarioOperation::StorageEvent {
                    action,
                    span_start,
                    span_end,
                } => self.record_storage_event(action, (*span_start, *span_end)),
                ScenarioOperation::NetworkEvent {
                    action,
                    span_start,
                    span_end,
                } => self.record_network_event(action, (*span_start, *span_end)),
                ScenarioOperation::Select {
                    branch_count,
                    span_start: _,
                    span_end: _,
                } => self
                    .events
                    .extend(crate::scheduler::select_events(*branch_count, self.options)),
                ScenarioOperation::RawNondeterminism {
                    operation,
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0102,
                    message:
                        "this replay path can change between runs; use a deterministic time/random facade"
                            .to_owned(),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "raw-nondeterminism".to_owned(),
                        label: Some(operation.clone()),
                        value: None,
                        io: None,
                    }],
                }),
                ScenarioOperation::UncontrolledEffect {
                    operation,
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0103,
                    message: format!(
                        "I do not know how to replay `{operation}` yet; model, record, or mark replay debt"
                    ),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "uncontrolled-effect".to_owned(),
                        label: Some(operation.clone()),
                        value: None,
                        io: None,
                    }],
                }),
                ScenarioOperation::ExternalBoundary {
                    crate_name,
                    call_path,
                    call_arguments,
                    return_type,
                    call_shape,
                    policy,
                    reason,
                    span_start,
                    span_end,
                } => self.record_external_boundary(
                    crate_name.clone(),
                    call_path.clone(),
                    call_arguments.clone(),
                    return_type.clone(),
                    call_shape.clone(),
                    policy.clone(),
                    reason.clone(),
                    (*span_start, *span_end),
                ),
                ScenarioOperation::Loop {
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0105,
                    message: format!(
                        "sim {} profile exceeded event budget {}",
                        self.options.sim_profile,
                        scheduler_budget(self.options)
                    ),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "budget-exceeded".to_owned(),
                        label: Some(self.options.sim_profile.clone()),
                        value: Some(scheduler_budget(self.options)),
                        io: None,
                    }],
                }),
            }
        }
    }

    fn discharge(&mut self, binding: &str, action: &str) {
        if let Some(obligation) = self
            .obligations
            .iter_mut()
            .rev()
            .find(|obligation| obligation.binding == binding && !obligation.is_discharged)
        {
            if action == "return"
                || action.starts_with("escape:")
                || action.starts_with("suppressed:")
                || obligation
                    .actions
                    .iter()
                    .any(|candidate| candidate == action)
            {
                obligation.is_discharged = true;
            }
        }
    }

    fn record_modeled_effect(&mut self, boundary: ModeledBoundary, span: (usize, usize)) {
        if !self.modeled_boundaries.contains(&boundary) {
            self.modeled_boundaries.push(boundary.clone());
        }
        self.events
            .extend(crate::scheduler::modeled_boundary_events(
                &boundary,
                self.options,
            ));
        if let Some(failure) = crate::scheduler::schedule_failure(
            &boundary,
            self.options,
            self.active_obligation(),
            span,
        ) {
            self.set_failure_once(failure);
        }
        if self.applied_injection {
            return;
        }
        self.applied_injection = true;
        let Some(inject) = self.options.inject.as_deref() else {
            return;
        };
        for hook in inject
            .split(',')
            .map(str::trim)
            .filter(|hook| !hook.is_empty())
        {
            match hook {
                "cancel" => self.record_cancel_hook(&boundary, span),
                "preempt" => self.events.push(ScenarioEvent {
                    kind: "failure-injection-preempt".to_owned(),
                    label: Some(boundary.as_str().to_owned()),
                    value: Some(self.options.seed),
                    io: None,
                }),
                "time-jump" | "timejump" => self.events.push(ScenarioEvent {
                    kind: "failure-injection-time-jump".to_owned(),
                    label: Some(boundary.as_str().to_owned()),
                    value: Some(self.options.seed.wrapping_add(60_000)),
                    io: None,
                }),
                "crash" => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0103,
                    message: format!(
                        "failure injection `crash` stopped modeled effect `{}`; record, model, or mark replay debt",
                        boundary.as_str()
                    ),
                    primary_start: span.0,
                    primary_end: span.1,
                    events: vec![ScenarioEvent {
                        kind: "failure-injection-crash".to_owned(),
                        label: Some(boundary.as_str().to_owned()),
                        value: None,
                        io: None,
                    }],
                }),
                _ => {}
            }
        }
    }

    fn record_transfer(&mut self, binding: &str, callee: &str, _span: (usize, usize)) {
        self.events.push(ScenarioEvent {
            kind: "obligation-transfer".to_owned(),
            label: Some(format!("{binding}->{callee}")),
            value: None,
            io: None,
        });
    }

    fn record_storage_event(&mut self, action: &str, span: (usize, usize)) {
        if !self
            .modeled_boundaries
            .contains(&ModeledBoundary::WardStorage)
        {
            self.modeled_boundaries.push(ModeledBoundary::WardStorage);
        }
        let transition = self.storage.apply_action(action, self.options.seed, span);
        self.events.extend(transition.events);
        if let Some(failure) = transition.failure {
            self.set_failure_once(failure);
        }
    }

    fn record_network_event(&mut self, action: &str, span: (usize, usize)) {
        if !self
            .modeled_boundaries
            .contains(&ModeledBoundary::WardNetwork)
        {
            self.modeled_boundaries.push(ModeledBoundary::WardNetwork);
        }
        let transition = self.network.apply_action(action, self.options.seed, span);
        self.events.extend(transition.events);
        if let Some(failure) = transition.failure {
            self.set_failure_once(failure);
        }
    }

    fn record_cancel_hook(&mut self, boundary: &ModeledBoundary, span: (usize, usize)) {
        let Some(obligation) = self
            .obligations
            .iter_mut()
            .rev()
            .find(|obligation| !obligation.is_discharged)
        else {
            self.events.push(ScenarioEvent {
                kind: "failure-injection-cancel".to_owned(),
                label: Some(boundary.as_str().to_owned()),
                value: None,
                io: None,
            });
            return;
        };

        obligation.drop_span = Some(span);
        let binding = obligation.binding.clone();
        let actions = obligation.actions.join(", ");
        self.set_failure_once(ScenarioFailure {
            code: KErrorCode::K0100,
            message: format!(
                "failure injection `cancel` cancelled active obligation `{binding}` at {}; discharge with {actions}",
                boundary.as_str()
            ),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![ScenarioEvent {
                kind: "failure-injection-cancel".to_owned(),
                label: Some(binding),
                value: None,
                io: None,
            }],
        });
    }

    fn record_external_boundary(
        &mut self,
        crate_name: String,
        call_path: Option<String>,
        call_arguments: Vec<ScenarioBoundaryCallArgument>,
        return_type: Option<String>,
        call_shape: ScenarioExternalCallShape,
        policy: BoundaryPolicyChoice,
        reason: Option<String>,
        span: (usize, usize),
    ) {
        if !is_replay_owned_boundary(&policy) && !self.opaque_boundaries.contains(&crate_name) {
            self.opaque_boundaries.push(crate_name.clone());
        }
        let event_label = boundary_event_label(&crate_name, call_path.as_deref(), span);
        let activity_capture = if policy == BoundaryPolicyChoice::Activity {
            Some(activity_result_capture(
                &crate_name,
                call_path.as_deref(),
                &call_arguments,
                &call_shape,
                reason.as_deref(),
                span,
                self.options.seed,
            ))
        } else {
            None
        };
        if !self.boundary_decisions.iter().any(|decision| {
            decision.crate_name == crate_name
                && decision.call_path == call_path
                && decision.call_arguments == call_arguments
                && decision.return_type == return_type
                && decision.call_shape == call_shape
                && decision.span_start == span.0
                && decision.span_end == span.1
        }) {
            self.boundary_decisions.push(BoundaryDecision {
                crate_name: crate_name.clone(),
                call_path: call_path.clone(),
                call_arguments: call_arguments.clone(),
                return_type: return_type.clone(),
                call_shape: call_shape.clone(),
                policy: policy.clone(),
                reason: reason.clone(),
                span_start: span.0,
                span_end: span.1,
                recorded_io: activity_capture.clone(),
            });
        }
        if is_replay_owned_boundary(&policy) {
            self.events.push(ScenarioEvent {
                kind: format!("boundary-{}", policy.as_str()),
                label: Some(event_label),
                value: Some(self.options.seed),
                io: None,
            });
            return;
        }
        if is_explicit_partial_boundary(&policy) {
            self.events.push(ScenarioEvent {
                kind: format!("boundary-{}", policy.as_str()),
                label: Some(event_label),
                value: Some(self.options.seed),
                io: activity_capture,
            });
            return;
        }
        self.set_failure_once(ScenarioFailure {
            code: KErrorCode::K0107,
            message: format!(
                "external replay boundary `{crate_name}` must choose one policy: typed, model, record, activity, stub, outside, opaque, debt"
            ),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![ScenarioEvent {
                kind: "boundary-policy-required".to_owned(),
                label: Some(event_label),
                value: None,
                io: None,
            }],
        });
    }

    fn set_failure_once(&mut self, failure: ScenarioFailure) {
        if self.failure.is_none() {
            self.failure = Some(failure);
        }
    }

    fn active_obligation(&self) -> Option<(&str, (usize, usize))> {
        self.obligations
            .iter()
            .rev()
            .find(|obligation| !obligation.is_discharged)
            .map(|obligation| (obligation.binding.as_str(), obligation.declaration_span))
    }

    fn finish(mut self, target: &str, lowered: LoweredScenario, agreement: &str) -> FullDepthRun {
        let liveness = self.liveness_failure();
        if self.failure.is_none() {
            self.failure = liveness;
        }
        if let Some(failure) = self.failure.as_ref() {
            self.events.extend(failure.events.clone());
        }
        self.events.extend(scheduler_events(self.options));
        let obligations = self
            .obligations
            .iter()
            .map(|obligation| RuntimeObligationSummary {
                binding: obligation.binding.clone(),
                type_name: obligation.type_name.clone(),
                actions: obligation.actions.clone(),
                is_discharged: obligation.is_discharged,
                declaration_span: obligation.declaration_span,
                drop_span: obligation.drop_span,
            })
            .collect::<Vec<_>>();
        let trace_hash = crate::digest::events_hash(&self.events);
        let scenario_ir_hash = crate::digest::operations_hash(&lowered.operations);
        let mut replay_guarantee = crate::coverage::replay_guarantee_for(
            &lowered.coverage,
            agreement,
            self.failure.as_ref().map(|failure| failure.code),
        );
        if !self.opaque_boundaries.is_empty()
            && !matches!(replay_guarantee, ReplayGuarantee::NotReplayable)
        {
            replay_guarantee = ReplayGuarantee::Partial;
        }
        FullDepthRun {
            target: target.to_owned(),
            profile: lowered.profile,
            replay_guarantee,
            events: self.events,
            failure: self.failure,
            coverage: lowered.coverage,
            digest: ExecutionDigest {
                semantic_engine: "driver-kir-scenario".to_owned(),
                harness_engine: "none".to_owned(),
                model_schema: crate::lower::MODEL_SCHEMA.to_owned(),
                schema_version: crate::lower::MODEL_SCHEMA_VERSION,
                scenario_ir_hash,
                operation_count: lowered.operations.len(),
                semantic_trace_hash: trace_hash,
                harness_trace_hash: String::new(),
                fuzz_driver_trace_hash: None,
                fuzz_driver_event_count: 0,
                agreement: agreement.to_owned(),
                generated_rust_hash: None,
                harness_manifest_hash: None,
                harness_exit_code: None,
                harness_event_count: 0,
            },
            modeled_boundaries: self.modeled_boundaries,
            opaque_boundaries: self.opaque_boundaries,
            obligations,
            boundary_decisions: self.boundary_decisions,
            harness_manifest: None,
        }
    }

    fn liveness_failure(&self) -> Option<ScenarioFailure> {
        let obligation = self
            .obligations
            .iter()
            .find(|obligation| !obligation.is_discharged)?;
        let span = obligation.drop_span.unwrap_or(obligation.declaration_span);
        let actions = obligation.actions.join(", ");
        let failure_mode = unresolved_failure_mode(&obligation.actions);
        Some(ScenarioFailure {
            code: KErrorCode::K0100,
            message: format!(
                "{failure_mode}: this path leaves `{}` open; finish with {actions} or pass it on as debt",
                obligation.binding
            ),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![ScenarioEvent {
                kind: "obligation-left-open".to_owned(),
                label: Some(obligation.binding.clone()),
                value: None,
                io: None,
            }],
        })
    }
}

fn unresolved_failure_mode(actions: &[String]) -> &'static str {
    if actions
        .iter()
        .any(|action| matches!(action.as_str(), "reply" | "reject" | "cancel"))
    {
        "reply-open"
    } else {
        "delivery-open"
    }
}

fn is_replay_owned_boundary(policy: &BoundaryPolicyChoice) -> bool {
    matches!(
        policy,
        BoundaryPolicyChoice::Model | BoundaryPolicyChoice::Record | BoundaryPolicyChoice::Stub
    )
}

fn is_explicit_partial_boundary(policy: &BoundaryPolicyChoice) -> bool {
    matches!(
        policy,
        BoundaryPolicyChoice::Typed
            | BoundaryPolicyChoice::Activity
            | BoundaryPolicyChoice::Outside
            | BoundaryPolicyChoice::Opaque
            | BoundaryPolicyChoice::Debt
    )
}

fn boundary_event_label(crate_name: &str, call_path: Option<&str>, span: (usize, usize)) -> String {
    format!("{}@{}..{}", call_path.unwrap_or(crate_name), span.0, span.1)
}

fn activity_result_capture(
    crate_name: &str,
    call_path: Option<&str>,
    call_arguments: &[ScenarioBoundaryCallArgument],
    call_shape: &ScenarioExternalCallShape,
    reason: Option<&str>,
    span: (usize, usize),
    seed: u64,
) -> BoundaryIoCapture {
    let call_path = call_path.unwrap_or(crate_name);
    let replay_key = crate::digest::stable_hash(&format!(
        "activity:{crate_name}:{call_path}:{}:{}:{seed}",
        span.0, span.1
    ));
    let mut request_fields = vec![
        BoundaryIoField {
            key: "capture_source".to_owned(),
            value: "semantic-activity-boundary".to_owned(),
        },
        BoundaryIoField {
            key: "crate".to_owned(),
            value: crate_name.to_owned(),
        },
        BoundaryIoField {
            key: "call_path".to_owned(),
            value: call_path.to_owned(),
        },
        BoundaryIoField {
            key: "call_shape".to_owned(),
            value: call_shape.as_str().to_owned(),
        },
        BoundaryIoField {
            key: "policy".to_owned(),
            value: "activity".to_owned(),
        },
        BoundaryIoField {
            key: "reason".to_owned(),
            value: reason.unwrap_or("").to_owned(),
        },
        BoundaryIoField {
            key: "source_span_start".to_owned(),
            value: span.0.to_string(),
        },
        BoundaryIoField {
            key: "source_span_end".to_owned(),
            value: span.1.to_string(),
        },
        BoundaryIoField {
            key: "argument_count".to_owned(),
            value: call_arguments.len().to_string(),
        },
    ];
    for argument in call_arguments {
        request_fields.push(BoundaryIoField {
            key: format!("argument_{}_source", argument.index),
            value: argument.source.clone(),
        });
    }
    let request_hash = crate::digest::stable_hash(&boundary_fields_material(
        "kobo-activity-request",
        &request_fields,
    ));
    let response_fields = vec![
        BoundaryIoField {
            key: "capture_source".to_owned(),
            value: "semantic-activity-boundary".to_owned(),
        },
        BoundaryIoField {
            key: "status".to_owned(),
            value: "activity-result-recorded".to_owned(),
        },
        BoundaryIoField {
            key: "result_mode".to_owned(),
            value: "metadata-only-outside-deterministic-replay".to_owned(),
        },
        BoundaryIoField {
            key: "external_internals_replayed".to_owned(),
            value: "false".to_owned(),
        },
    ];
    let response_hash = crate::digest::stable_hash(&boundary_fields_material(
        "kobo-activity-response",
        &response_fields,
    ));
    BoundaryIoCapture {
        mode: "activity-result-metadata".to_owned(),
        replay_key,
        request: BoundaryIoPayload {
            kind: "kobo-activity-request".to_owned(),
            fields: request_fields,
        },
        response: BoundaryIoPayload {
            kind: "kobo-activity-response".to_owned(),
            fields: response_fields,
        },
        request_hash,
        response_hash,
    }
}

fn boundary_fields_material(kind: &str, fields: &[BoundaryIoField]) -> String {
    let mut material = kind.to_owned();
    for field in fields {
        material.push('|');
        material.push_str(&field.key);
        material.push('=');
        material.push_str(&field.value);
    }
    material
}

pub(crate) fn scheduler_events(options: &ScenarioOptions) -> Vec<ScenarioEvent> {
    crate::scheduler::portfolio_events(options)
}

pub(crate) fn scheduler_budget(options: &ScenarioOptions) -> u64 {
    crate::scheduler::scheduler_budget(options)
}
