use super::json_schema::{boundary_decisions_json, events_json, span_json};
use super::trace_checks::{
    find_word, matching_brace, scrub_comments_and_strings, ward_blocks, WardBlock,
};
use super::*;

struct ModelEventExpectation {
    span: (usize, usize),
}

#[derive(Clone)]
struct ModelObligationExpectation {
    binding: String,
    span: (usize, usize),
}

#[derive(Clone)]
struct WardModelStep {
    kind: WardModelStepKind,
    span: (usize, usize),
}

#[derive(Clone)]
enum WardModelStepKind {
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
enum ModelBoundaryCall {
    WardTime,
    WardRandom,
    WardTask,
    WardTaskLocal,
    WardStorage,
    WardNetwork,
}

struct ModelComparisonSpec {
    steps: Vec<WardModelStep>,
    events: Vec<ModelEventExpectation>,
    obligations: Vec<ModelObligationExpectation>,
    source: &'static str,
}

struct WardModelRun {
    source: &'static str,
    engine: &'static str,
    seed: u64,
    ir: Vec<WardModelStep>,
    events: Vec<String>,
    states: Vec<(String, String)>,
    obligations: Vec<(String, String)>,
    scheduler_preset: Option<String>,
    steps_executed: usize,
}

struct ModelComparisonFailure {
    kind: &'static str,
    label: String,
    message: String,
    span: (usize, usize),
}

pub(super) fn apply_model_vs_implementation(
    source_path: &str,
    source: &str,
    seed: u64,
    run: &mut FullDepthRun,
) {
    if run.failure.is_some() {
        return;
    }
    let Some(failure) = model_comparison_failure(source_path, source, seed, run) else {
        return;
    };
    run.failure = Some(ScenarioFailure {
        code: KErrorCode::K0117,
        message: failure.message,
        primary_start: failure.span.0,
        primary_end: failure.span.1,
        events: vec![ScenarioEvent {
            kind: failure.kind.to_owned(),
            label: Some(failure.label),
            value: None,
            io: None,
        }],
    });
}

pub(crate) fn model_vs_implementation_json(
    source_path: &str,
    source: &str,
    seed: u64,
    run: &FullDepthRun,
) -> serde_json::Value {
    let spec = parse_model_comparison_spec(source);
    let boundary = model_boundary_policy_json(run);
    if !spec.requested() {
        return serde_json::json!({
            "status": "not_requested",
            "scheduler_seed": model_scheduler_seed_json(seed),
            "model_run": model_run_json(&execute_ward_model(&spec, seed)),
            "trace": {
                "status": "not_requested",
                "model_events": [],
                "implementation_events": implementation_event_kinds(run),
            },
            "obligations": {
                "status": "not_requested",
                "model_states": [],
                "implementation_states": implementation_obligation_states(run),
            },
            "boundary_policy": boundary,
        });
    }

    let model_run = execute_ward_model(&spec, seed);
    let trace = model_trace_comparison_json(source_path, source, run, &spec, &model_run);
    let obligations = model_obligation_comparison_json(source_path, source, run, &spec, &model_run);
    let status = if boundary["status"] == "downgraded" {
        "partial"
    } else if trace["status"] == "diverged" || obligations["status"] == "diverged" {
        "diverged"
    } else {
        "matched"
    };

    serde_json::json!({
        "status": status,
        "selection": {
            "requested": true,
            "source": spec.source,
        },
        "scheduler_seed": model_scheduler_seed_json(seed),
        "model_run": model_run_json(&model_run),
        "trace": trace,
        "obligations": obligations,
        "boundary_policy": boundary,
    })
}

fn model_comparison_failure(
    source_path: &str,
    source: &str,
    seed: u64,
    run: &FullDepthRun,
) -> Option<ModelComparisonFailure> {
    let spec = parse_model_comparison_spec(source);
    if !spec.requested() || model_boundary_downgrade(run) {
        return None;
    }
    let model_run = execute_ward_model(&spec, seed);
    if let Some(diff) = first_model_trace_difference(run, &model_run, &spec) {
        let model = diff.model.as_deref().unwrap_or("<missing model event>");
        let implementation = diff
            .implementation
            .as_deref()
            .unwrap_or("<missing implementation event>");
        return Some(ModelComparisonFailure {
            kind: "model-trace-divergence",
            label: diff.label(),
            message: format!(
                "model-vs-implementation trace diverged at event {}: model `{model}`, implementation `{implementation}`",
                diff.index
            ),
            span: diff.span,
        });
    }
    if let Some(diff) = first_model_obligation_difference(run, &model_run, &spec) {
        let model = diff.model.as_deref().unwrap_or("<missing model state>");
        let implementation = diff
            .implementation
            .as_deref()
            .unwrap_or("<missing implementation state>");
        return Some(ModelComparisonFailure {
            kind: "model-obligation-divergence",
            label: diff.binding.clone(),
            message: format!(
                "model-vs-implementation obligation `{}` diverged: model `{model}`, implementation `{implementation}`",
                diff.binding
            ),
            span: diff.span,
        });
    }
    let _ = source_path;
    None
}

fn model_trace_comparison_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    spec: &ModelComparisonSpec,
    model_run: &WardModelRun,
) -> serde_json::Value {
    let model_events = model_run.events.clone();
    let implementation_events = implementation_event_kinds(run);
    let first_difference = first_model_trace_difference(run, model_run, spec);
    let status = if first_difference.is_some() {
        "diverged"
    } else {
        "matched"
    };
    let source_span = first_difference
        .as_ref()
        .map(|difference| span_json(source_path, source, difference.span))
        .unwrap_or_else(|| {
            spec.events
                .first()
                .map(|expectation| span_json(source_path, source, expectation.span))
                .unwrap_or_else(|| span_json(source_path, source, (0, 0)))
        });
    serde_json::json!({
        "status": status,
        "model_events": model_events,
        "implementation_events": implementation_events,
        "first_difference": first_difference.as_ref().map(ModelTraceDifference::to_json),
        "source_span": source_span,
        "trace_excerpt": trace_excerpt_for_difference(run, first_difference.as_ref()),
    })
}

fn model_obligation_comparison_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    spec: &ModelComparisonSpec,
    model_run: &WardModelRun,
) -> serde_json::Value {
    let model_states = model_run
        .obligations
        .iter()
        .map(|(binding, state)| {
            serde_json::json!({
                "binding": binding,
                "state": state,
                "source_span": spec
                    .obligations
                    .iter()
                    .find(|expectation| expectation.binding == *binding)
                    .map(|expectation| span_json(source_path, source, expectation.span))
                    .unwrap_or_else(|| span_json(source_path, source, (0, 0))),
            })
        })
        .collect::<Vec<_>>();
    let implementation_states = implementation_obligation_states(run);
    let first_difference = first_model_obligation_difference(run, model_run, spec);
    let status = if first_difference.is_some() {
        "diverged"
    } else {
        "matched"
    };
    serde_json::json!({
        "status": status,
        "model_states": model_states,
        "implementation_states": implementation_states,
        "first_difference": first_difference.as_ref().map(ModelObligationDifference::to_json),
    })
}

struct ModelTraceDifference {
    index: usize,
    model: Option<String>,
    implementation: Option<String>,
    span: (usize, usize),
}

impl ModelTraceDifference {
    fn label(&self) -> String {
        format!(
            "event:{}:model={}:implementation={}",
            self.index,
            self.model.as_deref().unwrap_or("<missing>"),
            self.implementation.as_deref().unwrap_or("<missing>")
        )
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "index": self.index,
            "model": self.model,
            "implementation": self.implementation,
        })
    }
}

struct ModelObligationDifference {
    binding: String,
    model: Option<String>,
    implementation: Option<String>,
    span: (usize, usize),
}

impl ModelObligationDifference {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "binding": self.binding,
            "model": self.model,
            "implementation": self.implementation,
        })
    }
}

fn first_model_trace_difference(
    run: &FullDepthRun,
    model_run: &WardModelRun,
    spec: &ModelComparisonSpec,
) -> Option<ModelTraceDifference> {
    if model_run.events.is_empty() {
        return None;
    }
    let implementation_events = implementation_event_kinds(run);
    let max_len = model_run.events.len().max(implementation_events.len());
    for index in 0..max_len {
        let model = model_run.events.get(index).cloned();
        let implementation = implementation_events.get(index).cloned();
        if model != implementation {
            let span = spec
                .event_span(index)
                .or_else(|| spec.steps.last().map(|step| step.span))
                .unwrap_or((0, 0));
            return Some(ModelTraceDifference {
                index,
                model,
                implementation,
                span,
            });
        }
    }
    None
}

fn first_model_obligation_difference(
    run: &FullDepthRun,
    model_run: &WardModelRun,
    spec: &ModelComparisonSpec,
) -> Option<ModelObligationDifference> {
    for (binding, state) in &model_run.obligations {
        let implementation = run
            .obligations
            .iter()
            .find(|obligation| obligation.binding == *binding)
            .map(|obligation| {
                if obligation.is_discharged {
                    "discharged".to_owned()
                } else {
                    "leaked".to_owned()
                }
            });
        if implementation.as_deref() != Some(state.as_str()) {
            let span = spec.obligation_span(binding).unwrap_or((0, 0));
            return Some(ModelObligationDifference {
                binding: binding.clone(),
                model: Some(state.clone()),
                implementation,
                span,
            });
        }
    }
    None
}

fn trace_excerpt_for_difference(
    run: &FullDepthRun,
    difference: Option<&ModelTraceDifference>,
) -> Vec<serde_json::Value> {
    let Some(difference) = difference else {
        return Vec::new();
    };
    events_json(&run.events)
        .into_iter()
        .skip(difference.index.saturating_sub(2))
        .take(5)
        .collect()
}

fn implementation_event_kinds(run: &FullDepthRun) -> Vec<String> {
    run.events
        .iter()
        .filter(|event| is_model_trace_event(&event.kind))
        .map(|event| event.kind.clone())
        .collect()
}

fn is_model_trace_event(kind: &str) -> bool {
    kind.starts_with("deterministic-")
        || kind.ends_with("-boundary")
        || kind == "obligation-transfer"
}

fn implementation_obligation_states(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding,
                "state": if obligation.is_discharged { "discharged" } else { "leaked" },
                "actions": obligation.actions,
            })
        })
        .collect()
}

fn model_boundary_policy_json(run: &FullDepthRun) -> serde_json::Value {
    serde_json::json!({
        "status": if model_boundary_downgrade(run) { "downgraded" } else { "comparable" },
        "decisions": boundary_decisions_json(run),
    })
}

fn model_boundary_downgrade(run: &FullDepthRun) -> bool {
    run.boundary_decisions.iter().any(|decision| {
        matches!(
            decision.policy.as_str(),
            "opaque" | "outside" | "debt" | "stub"
        )
    })
}

fn model_scheduler_seed_json(seed: u64) -> serde_json::Value {
    serde_json::json!({
        "model": seed,
        "implementation": seed,
        "same_seed": true,
    })
}

fn parse_model_comparison_spec(source: &str) -> ModelComparisonSpec {
    let mut spec = ModelComparisonSpec {
        steps: Vec::new(),
        events: Vec::new(),
        obligations: Vec::new(),
        source: "not_requested",
    };
    parse_ward_model_specs(source, &mut spec);
    parse_legacy_model_directives(source, &mut spec);
    spec
}

fn parse_ward_model_specs(source: &str, spec: &mut ModelComparisonSpec) {
    for block in ward_blocks(source) {
        parse_structured_model_blocks(&block, spec);
        let mut offset = block.body_start;
        for raw_line in block.body.split_inclusive('\n') {
            let line = raw_line.trim_end_matches(['\r', '\n']);
            let trimmed = line.trim();
            let leading = line.find(trimmed).unwrap_or(0);
            let span = (offset + leading, offset + line.len());
            if let Some(rest) = trimmed.strip_prefix("model ") {
                spec.source = "ward_model";
                parse_model_directive(rest, span, spec);
            }
            offset += raw_line.len();
        }
    }
}

fn parse_structured_model_blocks(block: &WardBlock<'_>, spec: &mut ModelComparisonSpec) {
    let cleaned = scrub_comments_and_strings(block.body);
    let mut cursor = 0;
    while let Some(model_start) = find_word(&cleaned, cursor, "model") {
        let after_model = model_start + "model".len();
        let Some(open) = model_block_open(&cleaned, after_model) else {
            cursor = after_model;
            continue;
        };
        let Some(close) = matching_brace(&cleaned, open) else {
            break;
        };
        spec.source = "ward_model";
        let model_body = &block.body[open + 1..close];
        parse_structured_model_statements(model_body, block.body_start + open + 1, spec);
        cursor = close + 1;
    }
}

fn model_block_open(bytes: &[u8], from: usize) -> Option<usize> {
    let mut cursor = from;
    while let Some(byte) = bytes.get(cursor) {
        if *byte == b'{' {
            return Some(cursor);
        }
        if !byte.is_ascii_whitespace() {
            return None;
        }
        cursor += 1;
    }
    None
}

fn parse_structured_model_statements(
    model_body: &str,
    body_start: usize,
    spec: &mut ModelComparisonSpec,
) {
    let mut statement_start = 0usize;
    for statement in model_body.split_terminator(';') {
        let leading = statement
            .find(|ch: char| !ch.is_ascii_whitespace())
            .unwrap_or(0);
        let trimmed = statement.trim();
        let span = (
            body_start + statement_start + leading,
            body_start + statement_start + statement.len(),
        );
        parse_model_directive(trimmed, span, spec);
        statement_start += statement.len() + 1;
    }
}

fn parse_legacy_model_directives(source: &str, spec: &mut ModelComparisonSpec) {
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        let leading = line.find(trimmed).unwrap_or(0);
        let span = (offset + leading, offset + line.len());
        if let Some(rest) = model_directive(trimmed) {
            if spec.source == "not_requested" {
                spec.source = "legacy_directive";
            }
            parse_model_directive(rest, span, spec);
        }
        offset += raw_line.len();
    }
}

fn model_directive(line: &str) -> Option<&str> {
    line.strip_prefix("// kobo:model ")
}

fn parse_model_directive(rest: &str, span: (usize, usize), spec: &mut ModelComparisonSpec) {
    if let Some(event) = rest.strip_prefix("event ") {
        let event = event.trim().trim_matches([';', '}']);
        if !event.is_empty() {
            spec.steps.push(WardModelStep {
                kind: WardModelStepKind::EmitEvent(event.to_owned()),
                span,
            });
            spec.events.push(ModelEventExpectation { span });
        }
    } else if let Some(obligation) = rest.strip_prefix("obligation ") {
        let mut parts = obligation.split_whitespace();
        let Some(binding) = parts.next() else {
            return;
        };
        let Some(state) = parts.next() else {
            return;
        };
        if matches!(state, "discharged" | "leaked") {
            spec.steps.push(WardModelStep {
                kind: WardModelStepKind::SetObligation {
                    binding: binding.to_owned(),
                    state: state.to_owned(),
                },
                span,
            });
            spec.obligations.push(ModelObligationExpectation {
                binding: binding.to_owned(),
                span,
            });
        }
    } else if let Some((name, value)) = parse_model_state(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::SetState { name, value },
            span,
        });
    } else if let Some(preset) = parse_model_scheduler(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::SchedulerAssumption { preset },
            span,
        });
    } else if let Some((name, boundary)) = parse_model_transition(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::Transition { name, boundary },
            span,
        });
    } else if let Some(boundary) = parse_model_boundary_call(rest) {
        spec.steps.push(WardModelStep {
            kind: WardModelStepKind::BoundaryCall(boundary),
            span,
        });
    }
}

fn parse_model_state(rest: &str) -> Option<(String, String)> {
    let state = rest.strip_prefix("state ")?;
    let state = state.trim().trim_matches([';', '}']).trim();
    if let Some((name, value)) = state.split_once('=') {
        let name = name.trim();
        let value = value.trim().trim_matches('"');
        if !name.is_empty() && !value.is_empty() {
            return Some((name.to_owned(), value.to_owned()));
        }
    }
    let mut parts = state.split_whitespace();
    let name = parts.next()?;
    let value = parts.next()?;
    Some((name.to_owned(), value.to_owned()))
}

fn parse_model_scheduler(rest: &str) -> Option<String> {
    let preset = rest
        .strip_prefix("scheduler ")?
        .trim()
        .trim_matches([';', '}'])
        .trim();
    (!preset.is_empty()).then(|| preset.to_owned())
}

fn parse_model_transition(rest: &str) -> Option<(String, ModelBoundaryCall)> {
    let transition = rest.strip_prefix("transition ")?;
    let transition = transition.trim().trim_matches([';', '}']).trim();
    let (name, target) = transition.split_once("->")?;
    let name = name.trim();
    let boundary = parse_model_boundary_call(target.trim())?;
    (!name.is_empty()).then(|| (name.to_owned(), boundary))
}

fn parse_model_boundary_call(rest: &str) -> Option<ModelBoundaryCall> {
    let mut normalized = rest.trim().trim_matches([';', '}']).trim();
    normalized = normalized.strip_suffix("()").unwrap_or(normalized).trim();
    match normalized {
        "ward.time" => Some(ModelBoundaryCall::WardTime),
        "ward.random" => Some(ModelBoundaryCall::WardRandom),
        "ward.task" => Some(ModelBoundaryCall::WardTask),
        "ward.task.local" => Some(ModelBoundaryCall::WardTaskLocal),
        "ward.storage" => Some(ModelBoundaryCall::WardStorage),
        "ward.network" => Some(ModelBoundaryCall::WardNetwork),
        _ => None,
    }
}

impl ModelComparisonSpec {
    fn requested(&self) -> bool {
        !self.steps.is_empty()
    }

    fn event_span(&self, index: usize) -> Option<(usize, usize)> {
        self.steps
            .iter()
            .filter(|step| step.kind.emits_event())
            .nth(index)
            .map(|step| step.span)
    }

    fn obligation_span(&self, binding: &str) -> Option<(usize, usize)> {
        self.steps.iter().find_map(|step| match &step.kind {
            WardModelStepKind::SetObligation {
                binding: candidate, ..
            } if candidate == binding => Some(step.span),
            _ => None,
        })
    }
}

impl WardModelStepKind {
    fn emits_event(&self) -> bool {
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

    fn to_json(&self) -> serde_json::Value {
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
    fn as_str(self) -> &'static str {
        match self {
            Self::WardTime => "ward.time",
            Self::WardRandom => "ward.random",
            Self::WardTask => "ward.task",
            Self::WardTaskLocal => "ward.task.local",
            Self::WardStorage => "ward.storage",
            Self::WardNetwork => "ward.network",
        }
    }

    fn event_kind(self) -> &'static str {
        match self {
            Self::WardTime => "deterministic-time",
            Self::WardRandom => "deterministic-random",
            Self::WardTask | Self::WardTaskLocal => "deterministic-task",
            Self::WardStorage => "storage-boundary",
            Self::WardNetwork => "network-boundary",
        }
    }
}

fn execute_ward_model(spec: &ModelComparisonSpec, seed: u64) -> WardModelRun {
    let mut interpreter = WardModelInterpreter::new(seed);
    for step in &spec.steps {
        interpreter.execute(step);
    }
    let (events, states, obligations, scheduler_preset) = interpreter.finish();
    WardModelRun {
        source: spec.source,
        engine: if spec.source == "ward_model" {
            "ward-model-interpreter"
        } else {
            "legacy-directive-interpreter"
        },
        seed,
        ir: spec.steps.clone(),
        events,
        states,
        obligations,
        scheduler_preset,
        steps_executed: spec.steps.len(),
    }
}

struct WardModelInterpreter {
    seed: u64,
    events: Vec<String>,
    states: Vec<(String, String)>,
    obligations: Vec<(String, String)>,
    scheduler_preset: Option<String>,
}

impl WardModelInterpreter {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            events: Vec::new(),
            states: Vec::new(),
            obligations: Vec::new(),
            scheduler_preset: None,
        }
    }

    fn execute(&mut self, step: &WardModelStep) {
        match &step.kind {
            WardModelStepKind::EmitEvent(event) => self.events.push(event.clone()),
            WardModelStepKind::SetObligation { binding, state } => {
                self.set_obligation(binding, state);
            }
            WardModelStepKind::SetState { name, value } => {
                self.set_state(name, value);
            }
            WardModelStepKind::SchedulerAssumption { preset } => {
                self.scheduler_preset = Some(preset.clone());
            }
            WardModelStepKind::Transition { boundary, .. } => {
                self.events.push(boundary.event_kind().to_owned());
            }
            WardModelStepKind::BoundaryCall(boundary) => {
                self.events.push(boundary.event_kind().to_owned());
            }
        }
    }

    fn set_state(&mut self, name: &str, value: &str) {
        if let Some((_, existing)) = self
            .states
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
        {
            *existing = value.to_owned();
            return;
        }
        self.states.push((name.to_owned(), value.to_owned()));
    }

    fn set_obligation(&mut self, binding: &str, state: &str) {
        if let Some((_, existing)) = self
            .obligations
            .iter_mut()
            .find(|(candidate, _)| candidate == binding)
        {
            *existing = state.to_owned();
            return;
        }
        self.obligations
            .push((binding.to_owned(), state.to_owned()));
    }

    fn finish(
        self,
    ) -> (
        Vec<String>,
        Vec<(String, String)>,
        Vec<(String, String)>,
        Option<String>,
    ) {
        let _ = self.seed;
        (
            self.events,
            self.states,
            self.obligations,
            self.scheduler_preset,
        )
    }
}

fn model_run_json(model_run: &WardModelRun) -> serde_json::Value {
    serde_json::json!({
        "source": model_run.source,
        "engine": model_run.engine,
        "semantics": "typed_ward_model_ir",
        "seed": model_run.seed,
        "ir": model_run
            .ir
            .iter()
            .map(|step| {
                let mut value = step.kind.to_json();
                if let Some(object) = value.as_object_mut() {
                    object.insert("span_start".to_owned(), serde_json::json!(step.span.0));
                    object.insert("span_end".to_owned(), serde_json::json!(step.span.1));
                }
                value
            })
            .collect::<Vec<_>>(),
        "events": model_run.events,
        "states": model_run
            .states
            .iter()
            .map(|(name, value)| {
                serde_json::json!({
                    "name": name,
                    "value": value,
                })
            })
            .collect::<Vec<_>>(),
        "obligations": model_run
            .obligations
            .iter()
            .map(|(binding, state)| {
                serde_json::json!({
                    "binding": binding,
                    "state": state,
                })
            })
            .collect::<Vec<_>>(),
        "scheduler_assumptions": {
            "preset": model_run.scheduler_preset,
            "seed": model_run.seed,
            "same_as_implementation": true,
        },
        "steps_executed": model_run.steps_executed,
    })
}
