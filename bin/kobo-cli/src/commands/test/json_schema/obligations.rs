use super::*;

pub(crate) fn obligations_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<serde_json::Value> {
    run.obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "binding": obligation.binding,
                "type": obligation.type_name,
                "actions": obligation.actions,
                "discharged": obligation.is_discharged,
                "declaration_span": span_json(source_path, source, obligation.declaration_span),
                "drop_span": obligation.drop_span.map(|span| span_json(source_path, source, span)),
            })
        })
        .collect()
}

pub(crate) fn obligation_events_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for obligation in &run.obligations {
        events.push(serde_json::json!({
            "state": "created",
            "binding": obligation.binding,
            "type": obligation.type_name,
        }));
        if obligation.is_discharged {
            events.push(serde_json::json!({
                "state": "discharged",
                "binding": obligation.binding,
                "actions": obligation.actions,
            }));
        } else {
            events.push(serde_json::json!({
                "state": "leaked",
                "binding": obligation.binding,
                "failure_mode": unresolved_failure_mode(obligation),
            }));
        }
    }
    for event in &run.events {
        if event.kind == "obligation-transfer" {
            events.push(serde_json::json!({
                "state": "transferred",
                "binding": event.label,
            }));
        }
    }
    if run.failure.is_none() {
        events.push(serde_json::json!({
            "state": "returned",
            "binding": null,
        }));
    }
    events
}

pub(crate) fn trace_event_json(index: usize, event: &ScenarioEvent) -> serde_json::Value {
    serde_json::json!({
        "id": index,
        "kind": event.kind.clone(),
        "label": event.label.clone(),
        "value": event.value,
    })
}
