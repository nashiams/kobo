use kobo_ir::ScenarioBoundaryCallArgument;

use crate::core::ScenarioEvent;

#[derive(Clone)]
pub(super) struct BoundaryFacadeRecordCapture {
    pub(super) crate_name: String,
    pub(super) call_path: Option<String>,
    pub(super) call_shape: String,
    pub(super) policy: String,
    pub(super) reason: Option<String>,
    pub(super) return_type: Option<String>,
    pub(super) span: (usize, usize),
    pub(super) replay_key: String,
    pub(super) call_arguments: Vec<ScenarioBoundaryCallArgument>,
}

pub(super) fn record_boundary_event_statement(
    event: &ScenarioEvent,
    capture: &BoundaryFacadeRecordCapture,
) -> String {
    let label = event.label.as_deref().unwrap_or("");
    let value = event.value.unwrap_or_default();
    let call_path = capture.call_path.as_deref().unwrap_or(&capture.crate_name);
    let reason = capture.reason.as_deref().unwrap_or("");
    let return_payload = facade_return_payload(capture);
    format!(
        "crate::__kobo_emit_record_boundary_event({:?}, {:?}, {}, {:?}, {:?}, {:?}, {:?}, {:?}, {}, {}, {:?}, {:?}, &__kobo_call_arguments);",
        event.kind,
        label,
        value,
        capture.crate_name,
        call_path,
        capture.call_shape,
        capture.policy,
        reason,
        capture.span.0,
        capture.span.1,
        capture.replay_key,
        return_payload
    )
}

fn facade_return_payload(capture: &BoundaryFacadeRecordCapture) -> String {
    let return_path =
        capture
            .return_type
            .clone()
            .unwrap_or_else(|| match capture.call_shape.as_str() {
                "associated_function" | "method" => capture
                    .call_path
                    .as_deref()
                    .and_then(boundary_receiver_type_path)
                    .unwrap_or_else(|| capture.crate_name.clone()),
                _ => format!("{}::__KoboBoundaryValue", capture.crate_name),
            });
    format!("facade_return:{return_path}")
}

fn boundary_receiver_type_path(call_path: &str) -> Option<String> {
    let mut segments = call_path
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    (segments.len() >= 2).then(|| {
        let _method = segments.pop();
        segments.join("::")
    })
}
pub(super) fn record_boundary_runtime_support_source() -> &'static str {
    r#"
fn __kobo_json_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn __kobo_stable_hash(source: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn __kobo_field_json(key: &str, value: &str) -> String {
    format!(
        "{{\"key\":{},\"value\":{}}}",
        __kobo_json_string(key),
        __kobo_json_string(value)
    )
}

fn __kobo_payload_json(kind: &str, fields: &[(String, String)]) -> String {
    let fields = fields
        .iter()
        .map(|(key, value)| __kobo_field_json(key, value))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"kind\":{},\"fields\":[{}]}}",
        __kobo_json_string(kind),
        fields
    )
}

fn __kobo_payload_material(kind: &str, fields: &[(String, String)]) -> String {
    let mut material = String::from(kind);
    for (key, value) in fields {
        material.push('|');
        material.push_str(key);
        material.push('=');
        material.push_str(value);
    }
    material
}

fn __kobo_boundary_protocol(crate_name: &str, call_path: &str) -> &'static str {
    if crate_name.contains("sql")
        || call_path.contains("query")
        || call_path.contains("transaction")
        || call_path.contains("pool")
    {
        "database-like"
    } else {
        "http-like"
    }
}

fn __kobo_emit_record_boundary_event(
    kind: &str,
    label: &str,
    value: u64,
    crate_name: &str,
    call_path: &str,
    call_shape: &str,
    policy: &str,
    reason: &str,
    span_start: usize,
    span_end: usize,
    replay_key: &str,
    return_payload: &str,
    call_arguments: &[(usize, &'static str, String, String)],
) {
    let mut argument_fragments = Vec::new();
    let mut request_fields = vec![
        ("capture_source".to_owned(), "generated-boundary-facade-runtime".to_owned()),
        ("protocol".to_owned(), __kobo_boundary_protocol(crate_name, call_path).to_owned()),
        ("crate".to_owned(), crate_name.to_owned()),
        ("call_path".to_owned(), call_path.to_owned()),
        ("call_shape".to_owned(), call_shape.to_owned()),
        ("policy".to_owned(), policy.to_owned()),
        ("reason".to_owned(), reason.to_owned()),
        ("argument_count".to_owned(), call_arguments.len().to_string()),
        ("source_span_start".to_owned(), span_start.to_string()),
        ("source_span_end".to_owned(), span_end.to_string()),
    ];
    for (index, source, type_name, size) in call_arguments {
        request_fields.push((format!("argument_{index}_source"), (*source).to_owned()));
        request_fields.push((format!("argument_{index}_type"), type_name.clone()));
        request_fields.push((format!("argument_{index}_size"), size.clone()));
        argument_fragments.push(format!(
            "{{\"index\":{},\"source\":{},\"type\":{},\"size\":{}}}",
            index,
            __kobo_json_string(source),
            __kobo_json_string(type_name),
            size
        ));
    }
    let request_body = format!(
        "{{\"boundary\":{},\"operation\":{},\"arguments\":[{}]}}",
        __kobo_json_string(crate_name),
        __kobo_json_string(call_path),
        argument_fragments.join(",")
    );
    request_fields.push(("request_body".to_owned(), request_body));
    let request_hash = __kobo_stable_hash(&__kobo_payload_material(
        "kobo-boundary-request",
        &request_fields,
    ));

    let replay_result =
        __kobo_stable_hash(&format!("recorded-response:{replay_key}:{request_hash}:{return_payload}"));
    let response_body = format!(
        "{{\"recorded\":true,\"return\":{},\"replay_result\":{}}}",
        __kobo_json_string(return_payload),
        __kobo_json_string(&replay_result)
    );
    let response_fields = vec![
        ("capture_source".to_owned(), "generated-boundary-facade-runtime".to_owned()),
        ("status".to_owned(), "recorded".to_owned()),
        ("status_code".to_owned(), "200".to_owned()),
        ("replay_key".to_owned(), replay_key.to_owned()),
        ("replay_result".to_owned(), replay_result),
        ("return_payload".to_owned(), return_payload.to_owned()),
        ("response_body".to_owned(), response_body),
        ("external_internals_replayed".to_owned(), "false".to_owned()),
    ];
    let response_hash = __kobo_stable_hash(&__kobo_payload_material(
        "kobo-boundary-response",
        &response_fields,
    ));

    let request_json = __kobo_payload_json("kobo-boundary-request", &request_fields);
    let response_json = __kobo_payload_json("kobo-boundary-response", &response_fields);
    let io_json = format!(
        "{{\"mode\":\"recorded-boundary-io\",\"replay_key\":{},\"request\":{},\"response\":{},\"request_hash\":{},\"response_hash\":{}}}",
        __kobo_json_string(replay_key),
        request_json,
        response_json,
        __kobo_json_string(&request_hash),
        __kobo_json_string(&response_hash)
    );
    println!(
        "KOBO_EVENT:{{\"kind\":{},\"label\":{},\"value\":{},\"io\":{}}}",
        __kobo_json_string(kind),
        __kobo_json_string(label),
        value,
        io_json
    );
}

"#
}
