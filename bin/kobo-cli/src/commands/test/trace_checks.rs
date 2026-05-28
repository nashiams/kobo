use super::json_schema::{events_json, sanitize_name, span_json, trace_event_json};
use super::{
    parse_ward_syntax, serde_json, FileId, FullDepthRun, KErrorCode, ScenarioEvent,
    ScenarioFailure, WardItem,
};

#[derive(Clone, Copy)]
pub(super) enum TraceCheckDomain {
    Invariant,
    Temporal,
}

#[derive(Clone, Copy)]
pub(super) enum TraceCheckKind {
    Always,
    Eventually,
    Never,
}

#[derive(Clone)]
pub(super) struct TraceCheck {
    domain: TraceCheckDomain,
    name: String,
    kind: TraceCheckKind,
    event: String,
    span: (usize, usize),
    source: &'static str,
}

pub(super) struct EvaluatedTraceCheck {
    check: TraceCheck,
    status: &'static str,
    message: String,
    trace_excerpt: Vec<serde_json::Value>,
}

pub(super) struct WardBlock<'a> {
    pub(super) body: &'a str,
    pub(super) body_start: usize,
}

pub(super) fn apply_trace_checks(source_path: &str, source: &str, run: &mut FullDepthRun) {
    if run.failure.is_some() {
        return;
    }
    let Some(result) = evaluate_trace_checks(source_path, source, run)
        .into_iter()
        .find(|result| result.status == "failed")
    else {
        return;
    };
    let failure_kind = match result.check.domain {
        TraceCheckDomain::Invariant => "invariant-failure",
        TraceCheckDomain::Temporal => "temporal-failure",
    };
    run.failure = Some(ScenarioFailure {
        code: KErrorCode::K0108,
        message: result.message,
        primary_start: result.check.span.0,
        primary_end: result.check.span.1,
        events: vec![ScenarioEvent {
            kind: failure_kind.to_owned(),
            label: Some(result.check.name),
            value: None,
            io: None,
        }],
    });
}

pub(crate) fn trace_checks_json(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> serde_json::Value {
    let mut invariants = Vec::new();
    let mut temporals = Vec::new();
    for result in evaluate_trace_checks(source_path, source, run) {
        let value = serde_json::json!({
            "name": result.check.name,
            "kind": result.check.kind.as_str(),
            "event": result.check.event,
            "source": result.check.source,
            "status": result.status,
            "message": result.message,
            "source_span": span_json(source_path, source, result.check.span),
            "trace_excerpt": result.trace_excerpt,
        });
        match result.check.domain {
            TraceCheckDomain::Invariant => invariants.push(value),
            TraceCheckDomain::Temporal => temporals.push(value),
        }
    }
    serde_json::json!({
        "invariant_checks": invariants,
        "temporal_checks": temporals,
    })
}

pub(super) fn evaluate_trace_checks(
    source_path: &str,
    source: &str,
    run: &FullDepthRun,
) -> Vec<EvaluatedTraceCheck> {
    parse_trace_checks(source)
        .into_iter()
        .map(|check| evaluate_trace_check(source_path, source, run, check))
        .collect()
}

pub(super) fn evaluate_trace_check(
    _source_path: &str,
    _source: &str,
    run: &FullDepthRun,
    check: TraceCheck,
) -> EvaluatedTraceCheck {
    let matched_events = run
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind == check.event)
        .map(|(index, event)| trace_event_json(index, event))
        .collect::<Vec<_>>();
    let violating_events = run
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind != check.event)
        .map(|(index, event)| trace_event_json(index, event))
        .collect::<Vec<_>>();

    match check.kind {
        TraceCheckKind::Always if violating_events.is_empty() => EvaluatedTraceCheck {
            check,
            status: "passed",
            message: "all observed events satisfied the always check".to_owned(),
            trace_excerpt: matched_events,
        },
        TraceCheckKind::Always => EvaluatedTraceCheck {
            message: format!(
                "trace check `{}` expected only `{}` events but observed a different event",
                check.name, check.event
            ),
            check,
            status: "failed",
            trace_excerpt: violating_events.into_iter().take(5).collect(),
        },
        TraceCheckKind::Eventually if matched_events.is_empty() => EvaluatedTraceCheck {
            message: format!(
                "trace check `{}` expected event `{}` but it never occurred",
                check.name, check.event
            ),
            check,
            status: "failed",
            trace_excerpt: events_json(&run.events).into_iter().take(5).collect(),
        },
        TraceCheckKind::Eventually => EvaluatedTraceCheck {
            check,
            status: "passed",
            message: "required event occurred in the trace".to_owned(),
            trace_excerpt: matched_events.into_iter().take(5).collect(),
        },
        TraceCheckKind::Never if matched_events.is_empty() => EvaluatedTraceCheck {
            check,
            status: "passed",
            message: "forbidden event did not occur in the trace".to_owned(),
            trace_excerpt: Vec::new(),
        },
        TraceCheckKind::Never => EvaluatedTraceCheck {
            message: format!(
                "trace check `{}` forbids event `{}` but the event occurred",
                check.name, check.event
            ),
            check,
            status: "failed",
            trace_excerpt: matched_events.into_iter().take(5).collect(),
        },
    }
}

pub(super) fn parse_trace_checks(source: &str) -> Vec<TraceCheck> {
    let mut checks = parse_ward_trace_checks(source);
    checks.extend(parse_legacy_trace_check_directives(source));
    checks
}

pub(super) fn parse_ward_trace_checks(source: &str) -> Vec<TraceCheck> {
    parse_ward_syntax(source, FileId(0))
        .wards
        .into_iter()
        .flat_map(|ward| ward.items)
        .filter_map(ward_item_trace_check)
        .collect()
}

pub(super) fn ward_item_trace_check(item: WardItem) -> Option<TraceCheck> {
    match item {
        WardItem::Invariant(block) => {
            let (kind, event) = parse_trace_check_expression(&block.body)?;
            Some(TraceCheck {
                domain: TraceCheckDomain::Invariant,
                name: block.name,
                kind,
                event,
                span: (block.span.start as usize, block.span.end as usize),
                source: "ward_model",
            })
        }
        WardItem::Temporal(block) => {
            let (kind, event) = parse_trace_check_expression(&block.body)?;
            Some(TraceCheck {
                domain: TraceCheckDomain::Temporal,
                name: temporal_check_name(block.name, kind, &event),
                kind,
                event,
                span: (block.span.start as usize, block.span.end as usize),
                source: "ward_model",
            })
        }
        _ => None,
    }
}

pub(super) fn temporal_check_name(name: String, kind: TraceCheckKind, event: &str) -> String {
    if name.is_empty() {
        format!("temporal_{}_{}", kind.as_str(), sanitize_name(event))
    } else {
        name
    }
}

pub(super) fn parse_legacy_trace_check_directives(source: &str) -> Vec<TraceCheck> {
    let mut checks = Vec::new();
    let mut offset = 0;
    for raw_line in source.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        let Some(check_line) = legacy_trace_check_directive(trimmed) else {
            offset += raw_line.len();
            continue;
        };
        let leading = line.find(trimmed).unwrap_or(0);
        let span = (offset + leading, offset + line.len());
        if let Some(check) = parse_invariant_line(check_line, span, "legacy_directive") {
            checks.push(check);
        } else if let Some(check) = parse_temporal_line(check_line, span, "legacy_directive") {
            checks.push(check);
        }
        offset += raw_line.len();
    }
    checks
}

pub(super) fn legacy_trace_check_directive(line: &str) -> Option<&str> {
    line.strip_prefix("// kobo:").map(str::trim)
}

pub(super) fn parse_invariant_line(
    line: &str,
    span: (usize, usize),
    source: &'static str,
) -> Option<TraceCheck> {
    let rest = line.strip_prefix("invariant ")?;
    let name = rest
        .split(|ch: char| ch.is_ascii_whitespace() || ch == '{')
        .next()
        .filter(|value| !value.is_empty())?;
    let body_start = rest.find('{')? + 1;
    let body_end = rest.rfind('}')?;
    let (kind, event) = parse_trace_check_expression(rest[body_start..body_end].trim())?;
    Some(TraceCheck {
        domain: TraceCheckDomain::Invariant,
        name: name.to_owned(),
        kind,
        event,
        span,
        source,
    })
}

pub(super) fn parse_temporal_line(
    line: &str,
    span: (usize, usize),
    source: &'static str,
) -> Option<TraceCheck> {
    let rest = line.strip_prefix("temporal ")?;
    let (kind, event) = parse_trace_check_expression(rest.trim())?;
    Some(TraceCheck {
        domain: TraceCheckDomain::Temporal,
        name: format!("temporal_{}_{}", kind.as_str(), sanitize_name(&event)),
        kind,
        event,
        span,
        source,
    })
}

pub(super) fn parse_trace_check_expression(expression: &str) -> Option<(TraceCheckKind, String)> {
    let mut parts = expression.split_whitespace();
    let kind = TraceCheckKind::parse(parts.next()?)?;
    let event = parts.next()?.trim_matches([';', '}']).to_owned();
    if event.is_empty() {
        None
    } else {
        Some((kind, event))
    }
}

pub(super) fn ward_blocks(source: &str) -> Vec<WardBlock<'_>> {
    let cleaned = scrub_comments_and_strings(source);
    let mut blocks = Vec::new();
    let mut cursor = 0;
    while let Some(start) = find_word(&cleaned, cursor, "ward") {
        let Some(open) = find_byte(&cleaned, start, b'{') else {
            break;
        };
        let Some(close) = matching_brace(&cleaned, open) else {
            break;
        };
        blocks.push(WardBlock {
            body: &source[open + 1..close],
            body_start: open + 1,
        });
        cursor = close + 1;
    }
    blocks
}

pub(super) fn scrub_comments_and_strings(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut cleaned = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                let start = index;
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                blank_non_newlines(&mut cleaned, start, index);
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                blank_non_newlines(&mut cleaned, start, index);
            }
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index = (index + 2).min(bytes.len());
                    } else if bytes[index] == b'"' {
                        index += 1;
                        break;
                    } else {
                        index += 1;
                    }
                }
                blank_non_newlines(&mut cleaned, start, index);
            }
            _ => index += 1,
        }
    }
    cleaned
}

pub(super) fn blank_non_newlines(bytes: &mut [u8], start: usize, end: usize) {
    let bounded_end = end.min(bytes.len());
    for byte in &mut bytes[start..bounded_end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

pub(super) fn find_word(bytes: &[u8], from: usize, word: &str) -> Option<usize> {
    let needle = word.as_bytes();
    let mut cursor = from;
    while cursor + needle.len() <= bytes.len() {
        if &bytes[cursor..cursor + needle.len()] == needle {
            let end = cursor + needle.len();
            let before = bytes.get(cursor.saturating_sub(1));
            let after = bytes.get(end);
            if !before.is_some_and(is_ident_byte)
                && after.is_some_and(|byte| byte.is_ascii_whitespace())
            {
                return Some(cursor);
            }
        }
        cursor += 1;
    }
    None
}

pub(super) fn find_byte(bytes: &[u8], from: usize, needle: u8) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|byte| *byte == needle)
        .map(|relative| from + relative)
}

pub(super) fn matching_brace(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn is_ident_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || *byte == b'_'
}

impl TraceCheckKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "always" => Some(Self::Always),
            "eventually" => Some(Self::Eventually),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Eventually => "eventually",
            Self::Never => "never",
        }
    }
}
