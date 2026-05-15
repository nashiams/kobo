use kobo_errors::KErrorCode;
use kobo_ir::ScenarioProgram;

use crate::harness_manifest::HarnessManifest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineMode {
    SemanticOnly,
    HarnessOnly,
    Both,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioOptions {
    pub profile: String,
    pub seed: u64,
    pub inject: Option<String>,
    pub event_budget: Option<u64>,
}

impl Default for ScenarioOptions {
    fn default() -> Self {
        Self {
            profile: "checked".to_owned(),
            seed: 0,
            inject: None,
            event_budget: None,
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
    pub model_version: String,
    pub scenario_ir_hash: String,
    pub operation_count: usize,
    pub semantic_trace_hash: String,
    pub harness_trace_hash: String,
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
    pub policy: BoundaryPolicyChoice,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryPolicyChoice {
    Model,
    Record,
    Stub,
    Outside,
    Opaque,
    Debt,
    Unselected,
}

impl BoundaryPolicyChoice {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Record => "record",
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
}

impl ModeledBoundary {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
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
    MoveBinding {
        binding: String,
        span_start: usize,
        span_end: usize,
    },
    ModeledEffect {
        boundary: ModeledBoundary,
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
        span_start: usize,
        span_end: usize,
    },
    Loop {
        span_start: usize,
        span_end: usize,
    },
}

pub fn run_compiler_semantics(_source: &str, _target: &str) -> anyhow::Result<FullDepthRun> {
    anyhow::bail!(
        "source-based sim-core lowering is not available in production-depth mode; use run_semantics_from_program"
    )
}

pub fn run_semantics_from_program(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> anyhow::Result<FullDepthRun> {
    let lowered = crate::lower::lower_from_program(program, &options.profile);
    Ok(execute_lowered(&program.target, options, lowered))
}

pub fn run_full_depth(
    _source: &str,
    _target: &str,
    _mode: EngineMode,
    _options: ScenarioOptions,
) -> anyhow::Result<FullDepthRun> {
    anyhow::bail!(
        "source-based full-depth execution is not available; build a ScenarioProgram through kobo-driver"
    )
}

pub fn run_full_depth_from_program(
    program: &ScenarioProgram,
    generated_rust: &str,
    options: &ScenarioOptions,
    mode: EngineMode,
) -> anyhow::Result<FullDepthRun> {
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
                    self.discharge(binding, action);
                }
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
                ScenarioOperation::ModeledEffect {
                    boundary,
                    span_start,
                    span_end,
                } => self.record_modeled_effect(boundary.clone(), (*span_start, *span_end)),
                ScenarioOperation::RawNondeterminism {
                    operation,
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0102,
                    message:
                        "raw nondeterminism appears on replay path; use a deterministic time/random facade"
                            .to_owned(),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "raw-nondeterminism".to_owned(),
                        label: Some(operation.clone()),
                        value: None,
                    }],
                }),
                ScenarioOperation::UncontrolledEffect {
                    operation,
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0103,
                    message: format!(
                        "scenario cannot replay uncontrolled effect `{operation}`; model, record, or mark replay debt"
                    ),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "uncontrolled-effect".to_owned(),
                        label: Some(operation.clone()),
                        value: None,
                    }],
                }),
                ScenarioOperation::ExternalBoundary {
                    crate_name,
                    span_start,
                    span_end,
                } => self.record_external_boundary(crate_name.clone(), (*span_start, *span_end)),
                ScenarioOperation::Loop {
                    span_start,
                    span_end,
                } => self.set_failure_once(ScenarioFailure {
                    code: KErrorCode::K0105,
                    message: format!(
                        "sim quick profile exceeded event budget {}",
                        self.options.event_budget.unwrap_or(0)
                    ),
                    primary_start: *span_start,
                    primary_end: *span_end,
                    events: vec![ScenarioEvent {
                        kind: "budget-exceeded".to_owned(),
                        label: Some(self.options.event_budget.unwrap_or(0).to_string()),
                        value: self.options.event_budget,
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
            if obligation
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
            .push(modeled_effect_event(&boundary, self.options.seed));
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
                }),
                "time-jump" | "timejump" => self.events.push(ScenarioEvent {
                    kind: "failure-injection-time-jump".to_owned(),
                    label: Some(boundary.as_str().to_owned()),
                    value: Some(self.options.seed.wrapping_add(60_000)),
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
                    }],
                }),
                _ => {}
            }
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
            }],
        });
    }

    fn record_external_boundary(&mut self, crate_name: String, span: (usize, usize)) {
        if !self.opaque_boundaries.contains(&crate_name) {
            self.opaque_boundaries.push(crate_name.clone());
        }
        if !self
            .boundary_decisions
            .iter()
            .any(|decision| decision.crate_name == crate_name)
        {
            self.boundary_decisions.push(BoundaryDecision {
                crate_name: crate_name.clone(),
                policy: BoundaryPolicyChoice::Unselected,
                reason: None,
            });
        }
        self.set_failure_once(ScenarioFailure {
            code: KErrorCode::K0107,
            message: format!(
                "external replay boundary `{crate_name}` must choose one policy: model, record, stub, outside, opaque, debt"
            ),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![ScenarioEvent {
                kind: "boundary-policy-required".to_owned(),
                label: Some(crate_name),
                value: None,
            }],
        });
    }

    fn set_failure_once(&mut self, failure: ScenarioFailure) {
        if self.failure.is_none() {
            self.failure = Some(failure);
        }
    }

    fn finish(mut self, target: &str, lowered: LoweredScenario, agreement: &str) -> FullDepthRun {
        let liveness = self.liveness_failure();
        if self.failure.is_none() {
            self.failure = liveness;
        }
        if let Some(failure) = self.failure.as_ref() {
            self.events.extend(failure.events.clone());
        }
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
                model_version: crate::lower::MODEL_VERSION.to_owned(),
                scenario_ir_hash,
                operation_count: lowered.operations.len(),
                semantic_trace_hash: trace_hash,
                harness_trace_hash: String::new(),
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
        Some(ScenarioFailure {
            code: KErrorCode::K0100,
            message: format!(
                "checked scenario dropped `{}` without required action; discharge with {actions}",
                obligation.binding
            ),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![ScenarioEvent {
                kind: "liveness-token-drop".to_owned(),
                label: Some(obligation.binding.clone()),
                value: None,
            }],
        })
    }
}

fn modeled_effect_event(boundary: &ModeledBoundary, seed: u64) -> ScenarioEvent {
    match boundary {
        ModeledBoundary::WardTime => ScenarioEvent {
            kind: "deterministic-time".to_owned(),
            label: None,
            value: Some(seed.wrapping_mul(1_000).wrapping_add(17)),
        },
        ModeledBoundary::WardRandom => ScenarioEvent {
            kind: "deterministic-random".to_owned(),
            label: None,
            value: Some(seed.rotate_left(13) ^ 0x9e37_79b9_7f4a_7c15_u64),
        },
        ModeledBoundary::WardTask => ScenarioEvent {
            kind: "deterministic-task".to_owned(),
            label: Some("ward.task".to_owned()),
            value: Some(seed),
        },
    }
}
