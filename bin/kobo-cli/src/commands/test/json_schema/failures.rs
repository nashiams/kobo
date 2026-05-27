use super::*;

pub(crate) fn failure_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    primary_span: Option<String>,
) -> serde_json::Value {
    let Some(failure) = run.failure.as_ref() else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "code": failure.code.as_str(),
        "message": failure.message,
        "mode": run.failure
            .as_ref()
            .map(|_| run_failure_mode(run))
            .unwrap_or("none"),
        "primary_span": primary_span.unwrap_or_else(|| format!("{source_path}:1:1")),
        "related_spans": related_spans_json(source_path, source, run, failure),
    })
}

pub(crate) fn source_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    let mut spans = Vec::new();
    if let Some(failure) = run.failure.as_ref() {
        spans.push(span_json(
            source_path,
            source,
            (failure.primary_start, failure.primary_end),
        ));
    }
    for obligation in &run.obligations {
        spans.push(span_json(source_path, source, obligation.declaration_span));
        if let Some(drop_span) = obligation.drop_span {
            spans.push(span_json(source_path, source, drop_span));
        }
    }
    spans
}

pub(crate) fn run_failure_mode(run: &FullDepthRun) -> &'static str {
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "model-trace-divergence"))
    {
        return "model_trace_divergence";
    }
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "model-obligation-divergence"))
    {
        return "model_obligation_divergence";
    }
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "invariant-failure"))
    {
        return "invariant_failure";
    }
    if run
        .failure
        .as_ref()
        .is_some_and(|failure| scenario_failure_has_event(failure, "temporal-failure"))
    {
        return "temporal_failure";
    }
    if run
        .events
        .iter()
        .any(|event| event.kind == "lost-message" || event.kind == "storage-crash-after-write")
    {
        return "lost-message";
    }
    if run
        .obligations
        .iter()
        .any(|obligation| unresolved_failure_mode(obligation) == "unresolved-reply")
    {
        return "unresolved-reply";
    }
    "unresolved-delivery"
}

pub(crate) fn unresolved_failure_mode(
    obligation: &kobo_sim_core::RuntimeObligationSummary,
) -> &'static str {
    if obligation
        .actions
        .iter()
        .any(|action| action == "reply" || action == "reject" || action == "cancel")
    {
        "unresolved-reply"
    } else {
        "unresolved-delivery"
    }
}

pub(crate) fn escaped_obligations(run: &FullDepthRun) -> Vec<String> {
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged && obligation.drop_span.is_some())
        .map(|obligation| obligation.binding.clone())
        .collect()
}

pub(crate) fn related_spans_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
    failure: &ScenarioFailure,
) -> Vec<serde_json::Value> {
    if failure.code != KErrorCode::K0100 {
        return Vec::new();
    }
    run.obligations
        .iter()
        .filter(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            serde_json::json!({
                "label": format!("obligation `{}` declared here", obligation.binding),
                "span": span_json(source_path, source, obligation.declaration_span),
            })
        })
        .collect()
}

pub(crate) fn span_json(
    source_path: &str,
    source: &str,
    span: (usize, usize),
) -> serde_json::Value {
    serde_json::json!({
        "path": source_path,
        "line": one_based_line_for_offset(source, span.0),
        "start": span.0,
        "end": span.1.max(span.0 + 1),
        "mapped": span.1 > span.0,
        "snippet": line_snippet(source, span.0),
    })
}
