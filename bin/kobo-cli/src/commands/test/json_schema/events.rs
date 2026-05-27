use kobo_sim_core::{FullDepthRun, ScenarioEvent};

use super::boundaries::boundary_io_payload_json;

pub(crate) fn boundary_decisions_json(run: &FullDepthRun) -> Vec<serde_json::Value> {
    run.boundary_decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "crate": decision.crate_name,
                "call_path": decision.call_path,
                "call_arguments": decision.call_arguments,
                "return_type": decision.return_type,
                "call_shape": decision.call_shape.as_str(),
                "policy": decision.policy.as_str(),
                "reason": decision.reason,
                "source_span": {
                    "start": decision.span_start,
                    "end": decision.span_end,
                },
            })
        })
        .collect()
}

pub(crate) fn events_json(events: &[ScenarioEvent]) -> Vec<serde_json::Value> {
    events
        .iter()
        .enumerate()
        .map(|(id, event)| {
            let mut object = serde_json::Map::new();
            object.insert("id".to_owned(), serde_json::json!(id));
            object.insert("kind".to_owned(), event.kind.clone().into());
            object.insert("label".to_owned(), event.label.clone().into());
            object.insert("value".to_owned(), event.value.into());
            if let Some(io) = event.io.as_ref() {
                object.insert("io_capture".to_owned(), boundary_io_capture_json(io));
            }
            serde_json::Value::Object(object)
        })
        .collect()
}

pub(crate) fn boundary_io_capture_json(
    capture: &kobo_sim_core::BoundaryIoCapture,
) -> serde_json::Value {
    serde_json::json!({
        "mode": capture.mode.clone(),
        "replay_key": capture.replay_key.clone(),
        "request": boundary_io_payload_json(&capture.request),
        "response": boundary_io_payload_json(&capture.response),
        "request_hash": capture.request_hash.clone(),
        "response_hash": capture.response_hash.clone(),
    })
}

pub(crate) fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

pub(crate) fn one_based_line_for_offset(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

pub(crate) fn line_snippet(source: &str, offset: usize) -> String {
    let bounded = offset.min(source.len());
    let line_start = source[..bounded]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[bounded..]
        .find('\n')
        .map(|index| bounded + index)
        .unwrap_or(source.len());
    source[line_start..line_end].trim().to_owned()
}
