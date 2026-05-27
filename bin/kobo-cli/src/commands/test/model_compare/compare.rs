use kobo_errors::KErrorCode;
use kobo_sim_core::{FullDepthRun, ScenarioEvent, ScenarioFailure};

use super::super::json_schema::{boundary_decisions_json, events_json, span_json};
use super::execute::{execute_ward_model, model_run_json};
use super::parse::parse_model_comparison_spec;
use super::{ModelComparisonFailure, ModelComparisonSpec, WardModelRun};

pub(crate) fn apply_model_vs_implementation(
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
