#![allow(dead_code)]

use std::path::{Path, PathBuf};

use anyhow::Context;
use kobo_errors::KErrorCode;

use kobo_sim_core as sim_core;

#[derive(Clone, Debug)]
pub(super) struct ScenarioDocument {
    pub source: String,
    pub source_hash: String,
    pub scenarios: Vec<Scenario>,
    pub must_call_types: Vec<MustCallType>,
}

#[derive(Clone, Debug)]
pub(super) struct Scenario {
    pub name: String,
    pub profile: String,
    pub body: String,
    #[allow(dead_code)]
    pub body_start: usize,
}

#[derive(Clone, Debug)]
pub(super) struct MustCallType {
    pub type_name: String,
    pub actions: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SimulationOptions<'a> {
    pub profile: &'a str,
    pub seed: u64,
    pub inject: Option<&'a str>,
    pub event_budget: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct SimulationRun {
    pub scenario: Scenario,
    pub events: Vec<SimEvent>,
    pub failure: Option<ScenarioFailure>,
    pub backend: Backend,
    pub modeled_boundaries: Vec<ModeledBoundary>,
    pub opaque_boundaries: Vec<String>,
    pub obligations: Vec<RuntimeObligationSummary>,
    pub boundary_decisions: Vec<BoundaryDecision>,
    pub execution_digest: Option<ExecutionDigest>,
}

#[derive(Clone, Debug)]
pub(super) struct ScenarioFailure {
    pub code: KErrorCode,
    pub message: String,
    pub primary_start: usize,
    pub primary_end: usize,
    pub events: Vec<SimEvent>,
}

#[derive(Clone, Debug)]
pub(super) struct SimEvent {
    pub kind: String,
    pub label: Option<String>,
    pub value: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExecutionDigest {
    pub engine: String,
    pub model_version: String,
    pub scenario_ir_hash: String,
    pub operation_count: usize,
    pub event_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Backend {
    Loom,
    Shuttle,
    Proptest,
    Failpoints,
    DesignOnlyNetwork,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ModeledBoundary {
    WardTime,
    WardRandom,
    WardTask,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RuntimeObligationSummary {
    pub binding: String,
    pub type_name: String,
    pub actions: Vec<String>,
    pub is_discharged: bool,
    pub declaration_span: (usize, usize),
    pub drop_span: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BoundaryDecision {
    pub crate_name: String,
    pub policy: BoundaryPolicyChoice,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BoundaryPolicyChoice {
    Model,
    Record,
    Stub,
    Outside,
    Opaque,
    Debt,
    Unselected,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum TargetProfileShape {
    Sync,
    Async,
    StatefulInput,
    Failpoint,
    Network,
}

impl Backend {
    pub(super) const fn as_str(&self) -> &'static str {
        match self {
            Self::Loom => "loom",
            Self::Shuttle => "shuttle",
            Self::Proptest => "proptest",
            Self::Failpoints => "failpoints",
            Self::DesignOnlyNetwork => "network-design",
        }
    }
}

impl TargetProfileShape {
    pub(super) const fn as_profile(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
            Self::StatefulInput => "stateful-input",
            Self::Failpoint => "failpoint",
            Self::Network => "network",
        }
    }
}

impl ModeledBoundary {
    pub(super) const fn as_str(&self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
        }
    }
}

impl BoundaryPolicyChoice {
    pub(super) const fn as_str(&self) -> &'static str {
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

pub(super) fn boundary_policy_choices() -> [BoundaryPolicyChoice; 6] {
    [
        BoundaryPolicyChoice::Model,
        BoundaryPolicyChoice::Record,
        BoundaryPolicyChoice::Stub,
        BoundaryPolicyChoice::Outside,
        BoundaryPolicyChoice::Opaque,
        BoundaryPolicyChoice::Debt,
    ]
}

#[derive(Clone, Debug)]
pub(super) struct ScenarioProgram {
    pub(super) operations: Vec<ScenarioOperation>,
}

#[derive(Clone, Debug)]
pub(super) enum ScenarioOperation {
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

#[derive(Clone, Debug)]
struct RuntimeObligation {
    binding: String,
    type_name: String,
    actions: Vec<String>,
    declaration_span: (usize, usize),
    drop_span: Option<(usize, usize)>,
    is_discharged: bool,
}

#[derive(Clone, Debug)]
struct SimulationRuntime<'a> {
    scenario: &'a Scenario,
    seed: u64,
    event_budget: Option<u64>,
    failure_hooks: Vec<FailureHook>,
    has_applied_failure_hooks: bool,
    events: Vec<SimEvent>,
    modeled_boundaries: Vec<ModeledBoundary>,
    opaque_boundaries: Vec<String>,
    obligations: Vec<RuntimeObligation>,
    boundary_decisions: Vec<BoundaryDecision>,
    budget_failure: Option<ScenarioFailure>,
    raw_failure: Option<ScenarioFailure>,
    uncontrolled_failure: Option<ScenarioFailure>,
    cancel_failure: Option<ScenarioFailure>,
    injection_failure: Option<ScenarioFailure>,
    boundary_failure: Option<ScenarioFailure>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum FailureHook {
    Cancel,
    Preempt,
    TimeJump,
    Crash,
}

#[derive(Clone, Debug)]
struct LineInfo<'a> {
    text: &'a str,
    offset: usize,
}

#[derive(Clone, Debug)]
struct ScenarioHeader {
    profile: String,
}

pub(super) fn load_document(file: &Path) -> anyhow::Result<ScenarioDocument> {
    let source = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    Ok(parse_document(source))
}

pub(super) fn parse_document(source: String) -> ScenarioDocument {
    ScenarioDocument {
        source_hash: source_hash(&source),
        must_call_types: parse_must_call_types(&source),
        scenarios: parse_scenarios(&source),
        source,
    }
}

pub(super) fn run_quick(
    document: &ScenarioDocument,
    options: SimulationOptions<'_>,
) -> SimulationRun {
    run_quick_target(document, options, None)
}

pub(super) fn run_quick_target(
    document: &ScenarioDocument,
    options: SimulationOptions<'_>,
    target: Option<&str>,
) -> SimulationRun {
    if document.scenarios.is_empty() {
        return missing_scenario_run(options.profile, document.source.len());
    }

    let scenario = match target {
        Some(target) => document
            .scenarios
            .iter()
            .find(|scenario| scenario.name == target)
            .cloned(),
        None => document.scenarios.first().cloned(),
    };
    let Some(scenario) = scenario else {
        return target_not_found_run(options.profile, document.source.len());
    };

    let program = ScenarioProgram::from_document(document, &scenario);
    let mut runtime = SimulationRuntime::new(&scenario, options);
    runtime.execute(&program);
    let mut run = runtime.finish();
    run.execution_digest = Some(program.execution_digest(&run.events));
    run
}

pub(super) fn missing_scenario_run(profile: &str, source_len: usize) -> SimulationRun {
    let scenario = empty_scenario(profile);
    SimulationRun {
        scenario,
        events: Vec::new(),
        failure: Some(ScenarioFailure {
            code: KErrorCode::K0103,
            message: "no #[kobo::scenario] function found; add scenario metadata before running kobo test --sim quick".to_owned(),
            primary_start: 0,
            primary_end: source_len.min(1),
            events: Vec::new(),
        }),
        backend: Backend::Shuttle,
        modeled_boundaries: Vec::new(),
        opaque_boundaries: Vec::new(),
        obligations: Vec::new(),
        boundary_decisions: Vec::new(),
        execution_digest: None,
    }
}

pub(super) fn target_not_found_run(profile: &str, source_len: usize) -> SimulationRun {
    let scenario = empty_scenario(profile);
    SimulationRun {
        scenario,
        events: Vec::new(),
        failure: Some(ScenarioFailure {
            code: KErrorCode::K0103,
            message: "target scenario was not found in witness source".to_owned(),
            primary_start: 0,
            primary_end: source_len.min(1),
            events: Vec::new(),
        }),
        backend: Backend::Shuttle,
        modeled_boundaries: Vec::new(),
        opaque_boundaries: Vec::new(),
        obligations: Vec::new(),
        boundary_decisions: Vec::new(),
        execution_digest: None,
    }
}

pub(super) fn target_profile(
    document: &ScenarioDocument,
    target: &str,
    pinned: Option<&str>,
) -> String {
    if let Some(profile) = pinned {
        return profile.to_owned();
    }
    let source = target_profile_source(document, target);
    profile_shape_for_source(&source).as_profile().to_owned()
}

pub(super) fn profile_shape_for_source(source: &str) -> TargetProfileShape {
    let lower = source.to_ascii_lowercase();
    if has_network_shape(&lower) {
        TargetProfileShape::Network
    } else if has_failpoint_shape(&lower) {
        TargetProfileShape::Failpoint
    } else if has_stateful_input_shape(&lower) {
        TargetProfileShape::StatefulInput
    } else if has_async_shape(&lower) {
        TargetProfileShape::Async
    } else {
        TargetProfileShape::Sync
    }
}

pub(super) fn source_hash(source: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in source.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn target_profile_source(document: &ScenarioDocument, target: &str) -> String {
    if let Some(scenario) = document
        .scenarios
        .iter()
        .find(|scenario| scenario.name == target)
    {
        return format!("profile={}\n{}", scenario.profile, scenario.body);
    }
    target_function_source(&document.source, target).unwrap_or_else(|| document.source.clone())
}

fn target_function_source(source: &str, target: &str) -> Option<String> {
    for line in line_infos(source) {
        let trimmed = line.text.trim();
        if parse_function_name(trimmed).as_deref() != Some(target) {
            continue;
        }
        let (_, body) = extract_body(source, line.offset)?;
        return Some(format!("{trimmed}\n{body}"));
    }
    None
}

fn has_network_shape(lower: &str) -> bool {
    lower.contains("reqwest::")
        || lower.contains("hyper::")
        || lower.contains("std::net::")
        || lower.contains("::client::new")
}

fn has_failpoint_shape(lower: &str) -> bool {
    lower.contains("failpoint")
}

fn has_stateful_input_shape(lower: &str) -> bool {
    lower.contains("stateful")
        || lower.contains("property")
        || lower.contains("parse(")
        || lower.contains("parse_")
        || lower.contains("assert!")
}

fn has_async_shape(lower: &str) -> bool {
    lower.contains("profile=async")
        || lower.contains("async fn")
        || lower.contains("async move")
        || lower.contains("tokio::spawn")
        || lower.contains("spawn(")
        || lower.contains("select!")
}

pub(super) fn replay_token(source_identity: &str, seed: u64, run: &SimulationRun) -> String {
    let mut material = String::new();
    material.push_str(source_identity);
    material.push(':');
    material.push_str(&seed.to_string());
    material.push(':');
    material.push_str(run.backend.as_str());
    material.push(':');
    for event in &run.events {
        material.push_str(&event.kind);
        material.push('=');
        if let Some(label) = event.label.as_deref() {
            material.push_str(label);
        }
        material.push('/');
        if let Some(value) = event.value {
            material.push_str(&value.to_string());
        }
        material.push(';');
    }
    source_hash(&material)
}

pub(super) fn cli_relative_path(file: &Path) -> anyhow::Result<String> {
    let absolute = absolute_path(file)?;
    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let display_path = absolute.strip_prefix(&cwd).unwrap_or(&absolute);
    Ok(display_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

pub(super) fn resolve_witness_source(path: &str, witness_dir: &Path) -> PathBuf {
    let source_path = PathBuf::from(path);
    if source_path.is_absolute() {
        return source_path;
    }
    let base = std::env::current_dir()
        .ok()
        .or_else(|| witness_dir.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(source_path)
}

fn parse_must_call_types(source: &str) -> Vec<MustCallType> {
    let lines = line_infos(source);
    let mut types = Vec::new();
    let mut pending_actions = None;
    for line in lines {
        let trimmed = line.text.trim();
        if let Some(actions) = parse_must_call_actions(trimmed) {
            pending_actions = Some(actions);
            continue;
        }
        if let Some(actions) = pending_actions.take() {
            if let Some(type_name) = parse_struct_name(trimmed) {
                types.push(MustCallType { type_name, actions });
            }
        }
    }
    types
}

fn parse_scenarios(source: &str) -> Vec<Scenario> {
    let lines = line_infos(source);
    let mut scenarios = Vec::new();
    let mut pending_header = None;
    for line in lines {
        let trimmed = line.text.trim();
        if trimmed.contains("kobo::scenario") {
            pending_header = Some(ScenarioHeader {
                profile: extract_named_string(trimmed, "profile")
                    .unwrap_or_else(|| "async".to_owned()),
            });
            continue;
        }
        let Some(header) = pending_header.take() else {
            continue;
        };
        if let Some(name) = parse_function_name(trimmed) {
            if let Some((body_start, body)) = extract_body(source, line.offset) {
                scenarios.push(Scenario {
                    name,
                    profile: header.profile,
                    body,
                    body_start,
                });
            }
        }
    }
    scenarios
}

impl ScenarioProgram {
    fn from_document(document: &ScenarioDocument, scenario: &Scenario) -> Self {
        sim_core::lower::lower_from_parser(&document.source, &scenario.name, &scenario.profile)
            .map(|lowered| Self {
                operations: lowered
                    .operations
                    .into_iter()
                    .map(convert_core_operation)
                    .collect(),
            })
            .unwrap_or_else(|_| {
            let span_end = document.source.len().min(1);
            Self {
                operations: vec![ScenarioOperation::UncontrolledEffect {
                    operation: "semantic scenario lowering failed".to_owned(),
                    span_start: 0,
                    span_end,
                }],
            }
        })
    }

    fn execution_digest(&self, events: &[SimEvent]) -> ExecutionDigest {
        let mut ir_material = String::new();
        for operation in &self.operations {
            operation.push_fingerprint(&mut ir_material);
            ir_material.push('\n');
        }

        let mut event_material = String::new();
        for event in events {
            event_material.push_str(&event.kind);
            event_material.push('|');
            if let Some(label) = event.label.as_deref() {
                event_material.push_str(label);
            }
            event_material.push('|');
            if let Some(value) = event.value {
                event_material.push_str(&value.to_string());
            }
            event_material.push('\n');
        }

        ExecutionDigest {
            engine: "semantic-sim".to_owned(),
            model_version: sim_core::lower::MODEL_VERSION.to_owned(),
            scenario_ir_hash: source_hash(&ir_material),
            operation_count: self.operations.len(),
            event_hash: source_hash(&event_material),
        }
    }
}

fn convert_core_operation(operation: sim_core::ScenarioOperation) -> ScenarioOperation {
    match operation {
        sim_core::ScenarioOperation::CreateObligation {
            binding,
            type_name,
            actions,
            span_start,
            span_end,
        } => ScenarioOperation::CreateObligation {
            binding,
            type_name,
            actions,
            span_start,
            span_end,
        },
        sim_core::ScenarioOperation::Discharge { binding, action } => {
            ScenarioOperation::Discharge { binding, action }
        }
        sim_core::ScenarioOperation::MoveBinding {
            binding,
            span_start,
            span_end,
        } => ScenarioOperation::MoveBinding {
            binding,
            span_start,
            span_end,
        },
        sim_core::ScenarioOperation::ModeledEffect {
            boundary,
            span_start,
            span_end,
        } => ScenarioOperation::ModeledEffect {
            boundary: convert_core_boundary(boundary),
            span_start,
            span_end,
        },
        sim_core::ScenarioOperation::RawNondeterminism {
            operation,
            span_start,
            span_end,
        } => ScenarioOperation::RawNondeterminism {
            operation,
            span_start,
            span_end,
        },
        sim_core::ScenarioOperation::UncontrolledEffect {
            operation,
            span_start,
            span_end,
        } => ScenarioOperation::UncontrolledEffect {
            operation,
            span_start,
            span_end,
        },
        sim_core::ScenarioOperation::ExternalBoundary {
            crate_name,
            span_start,
            span_end,
        } => ScenarioOperation::ExternalBoundary {
            crate_name,
            span_start,
            span_end,
        },
        sim_core::ScenarioOperation::Loop {
            span_start,
            span_end,
        } => ScenarioOperation::Loop {
            span_start,
            span_end,
        },
    }
}

fn convert_core_boundary(boundary: sim_core::ModeledBoundary) -> ModeledBoundary {
    match boundary {
        sim_core::ModeledBoundary::WardTime => ModeledBoundary::WardTime,
        sim_core::ModeledBoundary::WardRandom => ModeledBoundary::WardRandom,
        sim_core::ModeledBoundary::WardTask => ModeledBoundary::WardTask,
    }
}

impl ScenarioOperation {
    fn push_fingerprint(&self, output: &mut String) {
        match self {
            Self::CreateObligation {
                binding,
                type_name,
                actions,
                span_start,
                span_end,
            } => {
                output.push_str("create:");
                output.push_str(binding);
                output.push(':');
                output.push_str(type_name);
                output.push(':');
                output.push_str(&actions.join("|"));
                output.push(':');
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
            Self::Discharge { binding, action } => {
                output.push_str("discharge:");
                output.push_str(binding);
                output.push(':');
                output.push_str(action);
            }
            Self::MoveBinding {
                binding,
                span_start,
                span_end,
            } => {
                output.push_str("move:");
                output.push_str(binding);
                output.push(':');
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
            Self::ModeledEffect {
                boundary,
                span_start,
                span_end,
            } => {
                output.push_str("modeled:");
                output.push_str(boundary.as_str());
                output.push(':');
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
            Self::RawNondeterminism {
                operation,
                span_start,
                span_end,
            } => {
                output.push_str("raw:");
                output.push_str(operation);
                output.push(':');
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
            Self::UncontrolledEffect {
                operation,
                span_start,
                span_end,
            } => {
                output.push_str("uncontrolled:");
                output.push_str(operation);
                output.push(':');
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
            Self::ExternalBoundary {
                crate_name,
                span_start,
                span_end,
            } => {
                output.push_str("boundary:");
                output.push_str(crate_name);
                output.push(':');
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
            Self::Loop {
                span_start,
                span_end,
            } => {
                output.push_str("loop:");
                output.push_str(&span_start.to_string());
                output.push(':');
                output.push_str(&span_end.to_string());
            }
        }
    }
}

impl<'a> SimulationRuntime<'a> {
    fn new(scenario: &'a Scenario, options: SimulationOptions<'a>) -> SimulationRuntime<'a> {
        SimulationRuntime {
            scenario,
            seed: options.seed,
            event_budget: options.event_budget,
            failure_hooks: ordered_failure_hooks(options.inject, options.seed),
            has_applied_failure_hooks: false,
            events: Vec::new(),
            modeled_boundaries: Vec::new(),
            opaque_boundaries: Vec::new(),
            obligations: Vec::new(),
            boundary_decisions: Vec::new(),
            budget_failure: None,
            raw_failure: None,
            uncontrolled_failure: None,
            cancel_failure: None,
            injection_failure: None,
            boundary_failure: None,
        }
    }

    fn execute(&mut self, program: &ScenarioProgram) {
        for operation in &program.operations {
            match operation {
                ScenarioOperation::CreateObligation {
                    binding,
                    type_name,
                    actions,
                    span_start,
                    span_end,
                } => self.create_obligation(
                    binding.clone(),
                    type_name.clone(),
                    actions.clone(),
                    (*span_start, *span_end),
                ),
                ScenarioOperation::Discharge { binding, action } => {
                    self.discharge_obligation(binding, action);
                }
                ScenarioOperation::MoveBinding {
                    binding,
                    span_start,
                    span_end,
                } => self.mark_binding_moved(binding, (*span_start, *span_end)),
                ScenarioOperation::ModeledEffect {
                    boundary,
                    span_start,
                    span_end,
                } => {
                    self.record_modeled_effect(boundary.clone(), (*span_start, *span_end));
                }
                ScenarioOperation::RawNondeterminism {
                    operation,
                    span_start,
                    span_end,
                } => self.record_raw_failure(operation, (*span_start, *span_end)),
                ScenarioOperation::UncontrolledEffect {
                    operation,
                    span_start,
                    span_end,
                } => self.record_uncontrolled_failure(operation, (*span_start, *span_end)),
                ScenarioOperation::ExternalBoundary {
                    crate_name,
                    span_start,
                    span_end,
                } => self.record_external_boundary(crate_name.clone(), (*span_start, *span_end)),
                ScenarioOperation::Loop {
                    span_start,
                    span_end,
                } => self.record_budget_failure((*span_start, *span_end)),
            }
        }
    }

    fn finish(self) -> SimulationRun {
        let liveness_failure = self.liveness_failure();
        let obligations = self.obligation_summaries();
        let mut events = self.events;
        let failure = self
            .budget_failure
            .or(self.raw_failure)
            .or(self.uncontrolled_failure)
            .or(self.injection_failure)
            .or(self.cancel_failure)
            .or(liveness_failure)
            .or(self.boundary_failure);
        if let Some(failure) = failure.as_ref() {
            events.extend(failure.events.clone());
        }
        SimulationRun {
            scenario: self.scenario.clone(),
            events,
            failure,
            backend: backend_for_profile(&self.scenario.profile),
            modeled_boundaries: self.modeled_boundaries,
            opaque_boundaries: self.opaque_boundaries,
            obligations,
            boundary_decisions: self.boundary_decisions,
            execution_digest: None,
        }
    }

    fn create_obligation(
        &mut self,
        binding: String,
        type_name: String,
        actions: Vec<String>,
        span: (usize, usize),
    ) {
        self.obligations.push(RuntimeObligation {
            binding,
            type_name,
            actions,
            declaration_span: span,
            drop_span: None,
            is_discharged: false,
        });
    }

    fn discharge_obligation(&mut self, binding: &str, action: &str) {
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

    fn mark_binding_moved(&mut self, binding: &str, span: (usize, usize)) {
        if let Some(obligation) = self
            .obligations
            .iter_mut()
            .rev()
            .find(|obligation| obligation.binding == binding)
        {
            obligation.drop_span = Some(span);
        }
    }

    fn record_modeled_effect(&mut self, boundary: ModeledBoundary, span: (usize, usize)) {
        if !self.modeled_boundaries.contains(&boundary) {
            self.modeled_boundaries.push(boundary.clone());
        }
        let hooks = self.failure_hooks_for_effect();
        self.events.push(modeled_effect_event(
            boundary.clone(),
            self.seed,
            hooks.contains(&FailureHook::TimeJump),
        ));
        for hook in hooks {
            self.apply_failure_hook(hook, &boundary, span);
        }
    }

    fn failure_hooks_for_effect(&mut self) -> Vec<FailureHook> {
        if self.has_applied_failure_hooks {
            return Vec::new();
        }
        self.has_applied_failure_hooks = true;
        self.failure_hooks.clone()
    }

    fn apply_failure_hook(
        &mut self,
        hook: FailureHook,
        boundary: &ModeledBoundary,
        span: (usize, usize),
    ) {
        match hook {
            FailureHook::Cancel => self.record_cancel_hook(boundary, span),
            FailureHook::Preempt => self.events.push(SimEvent {
                kind: "failure-injection-preempt".to_owned(),
                label: Some(boundary.as_str().to_owned()),
                value: Some(self.seed),
            }),
            FailureHook::TimeJump => self.events.push(SimEvent {
                kind: "failure-injection-time-jump".to_owned(),
                label: Some(boundary.as_str().to_owned()),
                value: Some(self.seed.wrapping_add(60_000)),
            }),
            FailureHook::Crash => self.record_crash_hook(boundary, span),
        }
    }

    fn record_cancel_hook(&mut self, boundary: &ModeledBoundary, span: (usize, usize)) {
        let Some(obligation) = self
            .obligations
            .iter_mut()
            .rev()
            .find(|obligation| !obligation.is_discharged)
        else {
            self.events.push(SimEvent {
                kind: "failure-injection-cancel".to_owned(),
                label: Some(boundary.as_str().to_owned()),
                value: None,
            });
            return;
        };

        obligation.drop_span = Some(span);
        let binding = obligation.binding.clone();
        let actions = obligation.actions.join(", ");
        if self.cancel_failure.is_none() {
            self.cancel_failure = Some(ScenarioFailure {
                code: KErrorCode::K0100,
                message: format!(
                    "failure injection `cancel` cancelled active obligation `{binding}` at {}; discharge with {actions}",
                    boundary.as_str()
                ),
                primary_start: span.0,
                primary_end: span.1,
                events: vec![SimEvent {
                    kind: "failure-injection-cancel".to_owned(),
                    label: Some(binding),
                    value: None,
                }],
            });
        }
    }

    fn record_crash_hook(&mut self, boundary: &ModeledBoundary, span: (usize, usize)) {
        self.events.push(SimEvent {
            kind: "failure-injection-crash".to_owned(),
            label: Some(boundary.as_str().to_owned()),
            value: None,
        });
        if self.injection_failure.is_none() {
            self.injection_failure = Some(ScenarioFailure {
                code: KErrorCode::K0103,
                message: format!(
                    "failure injection `crash` stopped modeled effect `{}`; record, model, or mark replay debt",
                    boundary.as_str()
                ),
                primary_start: span.0,
                primary_end: span.1,
                events: vec![SimEvent {
                    kind: "failure-injection-crash".to_owned(),
                    label: Some(boundary.as_str().to_owned()),
                    value: None,
                }],
            });
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
            events: vec![SimEvent {
                kind: "liveness-token-drop".to_owned(),
                label: Some(obligation.binding.clone()),
                value: None,
            }],
        })
    }

    fn obligation_summaries(&self) -> Vec<RuntimeObligationSummary> {
        self.obligations
            .iter()
            .map(|obligation| RuntimeObligationSummary {
                binding: obligation.binding.clone(),
                type_name: obligation.type_name.clone(),
                actions: obligation.actions.clone(),
                is_discharged: obligation.is_discharged,
                declaration_span: obligation.declaration_span,
                drop_span: obligation.drop_span,
            })
            .collect()
    }
}

fn modeled_effect_event(boundary: ModeledBoundary, seed: u64, has_time_jump: bool) -> SimEvent {
    match boundary {
        ModeledBoundary::WardTime => SimEvent {
            kind: "deterministic-time".to_owned(),
            label: None,
            value: Some(
                seed.wrapping_mul(1_000)
                    .wrapping_add(17)
                    .wrapping_add(if has_time_jump { 60_000 } else { 0 }),
            ),
        },
        ModeledBoundary::WardRandom => SimEvent {
            kind: "deterministic-random".to_owned(),
            label: None,
            value: Some(seed.rotate_left(13) ^ 0x9e37_79b9_7f4a_7c15_u64),
        },
        ModeledBoundary::WardTask => SimEvent {
            kind: "deterministic-task".to_owned(),
            label: Some("ward.task".to_owned()),
            value: Some(seed),
        },
    }
}

impl<'a> SimulationRuntime<'a> {
    fn record_raw_failure(&mut self, operation: &str, span: (usize, usize)) {
        if self.raw_failure.is_some() {
            return;
        }
        self.raw_failure = Some(ScenarioFailure {
            code: KErrorCode::K0102,
            message:
                "raw nondeterminism appears on replay path; use a deterministic time/random facade"
                    .to_owned(),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![SimEvent {
                kind: "raw-nondeterminism".to_owned(),
                label: Some(operation.to_owned()),
                value: None,
            }],
        });
    }

    fn record_uncontrolled_failure(&mut self, operation: &str, span: (usize, usize)) {
        if self.uncontrolled_failure.is_some() {
            return;
        }
        self.uncontrolled_failure = Some(ScenarioFailure {
            code: KErrorCode::K0103,
            message: format!(
                "scenario cannot replay uncontrolled effect `{operation}`; model, record, or mark replay debt"
            ),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![SimEvent {
                kind: "uncontrolled-effect".to_owned(),
                label: Some(operation.to_owned()),
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
        if self.boundary_failure.is_none() {
            let choices = "model, record, stub, outside, opaque, debt";
            self.boundary_failure = Some(ScenarioFailure {
                code: KErrorCode::K0107,
                message: format!(
                    "external replay boundary `{crate_name}` must choose one policy: {choices}",
                ),
                primary_start: span.0,
                primary_end: span.1,
                events: vec![SimEvent {
                    kind: "boundary-policy-required".to_owned(),
                    label: Some(crate_name),
                    value: None,
                }],
            });
        }
    }

    fn record_budget_failure(&mut self, span: (usize, usize)) {
        if self.budget_failure.is_some() {
            return;
        }
        let budget = self.event_budget.unwrap_or(0);
        self.budget_failure = Some(ScenarioFailure {
            code: KErrorCode::K0105,
            message: format!("sim quick profile exceeded event budget {budget}"),
            primary_start: span.0,
            primary_end: span.1,
            events: vec![SimEvent {
                kind: "budget-exceeded".to_owned(),
                label: Some(budget.to_string()),
                value: Some(budget),
            }],
        });
    }
}

fn ordered_failure_hooks(inject: Option<&str>, seed: u64) -> Vec<FailureHook> {
    let Some(inject) = inject else {
        return Vec::new();
    };
    let mut hooks = inject
        .split(',')
        .filter_map(|hook| FailureHook::from_label(hook.trim()))
        .collect::<Vec<_>>();
    if hooks.len() > 1 {
        let rotate_by = (seed as usize) % hooks.len();
        hooks.rotate_left(rotate_by);
    }
    hooks
}

impl FailureHook {
    fn from_label(label: &str) -> Option<Self> {
        match label {
            "cancel" => Some(Self::Cancel),
            "preempt" => Some(Self::Preempt),
            "time-jump" | "timejump" => Some(Self::TimeJump),
            "crash" => Some(Self::Crash),
            _ => None,
        }
    }
}

fn empty_scenario(profile: &str) -> Scenario {
    Scenario {
        name: "<missing>".to_owned(),
        profile: profile.to_owned(),
        body: String::new(),
        body_start: 0,
    }
}

fn parse_must_call_actions(line: &str) -> Option<Vec<String>> {
    let start = line.find("kobo::must_call(")? + "kobo::must_call(".len();
    let end = line[start..].find(')')? + start;
    Some(
        line[start..end]
            .split('|')
            .map(str::trim)
            .filter(|action| !action.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn parse_struct_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix("struct ")?;
    ident_prefix(rest)
}

fn parse_function_name(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("async fn ")
        .or_else(|| line.strip_prefix("fn "))
        .or_else(|| line.strip_prefix("pub async fn "))
        .or_else(|| line.strip_prefix("pub fn "))?;
    ident_prefix(rest)
}

fn extract_body(source: &str, signature_start: usize) -> Option<(usize, String)> {
    let open_relative = source[signature_start..].find('{')?;
    let body_start = signature_start + open_relative + 1;
    let mut depth = 1usize;
    for (relative, ch) in source[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some((
                        body_start,
                        source[body_start..body_start + relative].to_owned(),
                    ));
                }
            }
            _ => {}
        }
    }
    None
}

fn extract_named_string(line: &str, key: &str) -> Option<String> {
    let key_start = line.find(key)?;
    let after_key = &line[key_start + key.len()..];
    let quote_start = after_key.find('"')?;
    let rest = &after_key[quote_start + 1..];
    let quote_end = rest.find('"')?;
    Some(rest[..quote_end].to_owned())
}

fn ident_prefix(input: &str) -> Option<String> {
    let ident = input
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    (!ident.is_empty()).then_some(ident)
}

pub(super) fn backend_for_profile(profile: &str) -> Backend {
    match profile {
        "sync" => Backend::Loom,
        "stateful-input" => Backend::Proptest,
        "failpoint" => Backend::Failpoints,
        "network" | "network-design" => Backend::DesignOnlyNetwork,
        _ => Backend::Shuttle,
    }
}

fn line_infos(source: &str) -> Vec<LineInfo<'_>> {
    let mut lines = Vec::new();
    let mut offset = 0usize;
    for line in source.lines() {
        lines.push(LineInfo { text: line, offset });
        offset += line.len() + 1;
    }
    lines
}

fn absolute_path(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("failed to determine current directory")?
            .join(path))
    }
}
