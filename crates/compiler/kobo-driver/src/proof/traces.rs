use super::{
    lifecycle_template_version, line_snippet, one_based_line_for_offset, source_span_from_kobo,
    template_by_binding, trace_material_hash, BTreeMap, BTreeSet, CoreTraceEvent,
    GeneratedTraceEvent, HashEvidence, KoboSourceMap, KoboSpan, LoweringTraceEvent,
    ObligationEvent, ObligationEventKind, ScenarioOpKind, ScenarioProgram, SourceMapAnchorEvidence,
    SourceMapAnchorStatus, SourceSpan, TraceEventKind,
};

pub(super) fn core_trace_evidence(
    program: &ScenarioProgram,
    obligation_events: &[ObligationEvent],
    source_path: &str,
    source: &str,
) -> Vec<CoreTraceEvent> {
    let template_by_binding = template_by_binding(program);
    let events_by_statement = obligation_events
        .iter()
        .filter_map(|event| statement_index_from_event(&event.id).map(|index| (index, event)))
        .collect::<BTreeMap<_, _>>();
    let has_traceable_obligation_events = obligation_events
        .iter()
        .any(|event| trace_event_kind(&event.kind).is_some());
    program
        .operations
        .iter()
        .enumerate()
        .filter_map(|(operation_index, operation)| {
            if !has_traceable_obligation_events && matches!(operation.kind, ScenarioOpKind::Return)
            {
                return None;
            }
            let event = events_by_statement.get(&operation_index).copied();
            let kind = event
                .and_then(|event| trace_event_kind(&event.kind))
                .or_else(|| trace_event_kind_from_operation(&operation.kind))?;
            let template = event
                .and_then(|event| event.binding.as_deref())
                .and_then(|binding| template_by_binding.get(binding));
            let binding = event.and_then(|event| event.binding.clone());
            let source_span = event
                .map(|event| event.source_span.clone())
                .unwrap_or_else(|| source_span_from_kobo(source_path, source, operation.span));
            let id = event
                .map(|event| format!("core-{}-{}", program.target, event.id))
                .unwrap_or_else(|| format!("core-{}-term-{operation_index}", program.target));
            Some(CoreTraceEvent {
                id,
                kind,
                binding,
                order: operation_index as u64,
                source_span,
                template_id: template.map(|template| template.id.clone()),
                template_version: template.map(|template| lifecycle_template_version(template)),
            })
        })
        .collect()
}

pub(super) fn statement_index_from_event(event_id: &str) -> Option<usize> {
    event_id.strip_prefix("stmt-")?.parse().ok()
}

pub(super) fn trace_event_kind_from_operation(kind: &ScenarioOpKind) -> Option<TraceEventKind> {
    match kind {
        ScenarioOpKind::Return => Some(TraceEventKind::Return),
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(trace_event_kind_from_terminator(kind)),
        _ => None,
    }
}

pub(super) fn trace_event_kind_from_terminator(
    kind: &kobo_ir::ScenarioCoreTerminatorKind,
) -> TraceEventKind {
    match kind {
        kobo_ir::ScenarioCoreTerminatorKind::Return => TraceEventKind::Return,
        kobo_ir::ScenarioCoreTerminatorKind::ErrorExit => TraceEventKind::ErrorExit,
        kobo_ir::ScenarioCoreTerminatorKind::Panic => TraceEventKind::Panic,
        kobo_ir::ScenarioCoreTerminatorKind::Await => TraceEventKind::Cancel,
        kobo_ir::ScenarioCoreTerminatorKind::OpaqueBoundary => TraceEventKind::OpaqueBoundary,
    }
}

pub(super) fn generated_trace_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    core_trace: &[CoreTraceEvent],
    source_map: Option<&KoboSourceMap>,
) -> Vec<GeneratedTraceEvent> {
    let Some(source_map) = source_map else {
        return Vec::new();
    };
    let mut used_lowering_events = BTreeSet::new();
    core_trace
        .iter()
        .filter_map(|event| {
            let (index, lowering_event) =
                lowering_trace_event_for_core(source_map, program, event, &used_lowering_events)?;
            used_lowering_events.insert(index);
            Some(GeneratedTraceEvent {
                id: format!("generated-{}", event.id),
                core_event_id: lowering_event.core_event_id.clone(),
                kind: event.kind.clone(),
                binding: event.binding.clone(),
                order: lowering_event.order,
                source_map_anchor: SourceMapAnchorEvidence {
                    id: lowering_event.source_map_entry_id.clone(),
                    status: SourceMapAnchorStatus::Mapped,
                    generated_span: generated_source_span(source_map, lowering_event),
                    kobo_span: source_map_anchor_span_from_kobo_span(
                        source_path,
                        source,
                        &lowering_event.kobo_span,
                    ),
                },
                lowering_phase: lowering_event.lowering_phase.clone(),
                template_id: lowering_event
                    .template_id
                    .clone()
                    .or(event.template_id.clone()),
                template_version: lowering_event
                    .template_version
                    .clone()
                    .or(event.template_version.clone()),
            })
        })
        .collect()
}

pub(super) fn trace_hashes(
    core_trace: &[CoreTraceEvent],
    generated_trace: &[GeneratedTraceEvent],
) -> Result<Vec<HashEvidence>, serde_json::Error> {
    Ok(vec![
        HashEvidence {
            id: "core_obligation_trace".to_owned(),
            hash: trace_material_hash(&core_trace)?,
        },
        HashEvidence {
            id: "generated_rust_trace".to_owned(),
            hash: trace_material_hash(&generated_trace)?,
        },
    ])
}

pub(super) fn trace_event_kind(kind: &ObligationEventKind) -> Option<TraceEventKind> {
    match kind {
        ObligationEventKind::Create => Some(TraceEventKind::Create),
        ObligationEventKind::Discharge => Some(TraceEventKind::Discharge),
        ObligationEventKind::Transfer => Some(TraceEventKind::Transfer),
        ObligationEventKind::Move => Some(TraceEventKind::Move),
        ObligationEventKind::Escape => Some(TraceEventKind::Escape),
        ObligationEventKind::BranchUnresolved
        | ObligationEventKind::UnsupportedContainer
        | ObligationEventKind::Call => None,
    }
}

pub(super) fn lowering_trace_event_for_core<'a>(
    source_map: &'a KoboSourceMap,
    program: &ScenarioProgram,
    event: &CoreTraceEvent,
    used: &BTreeSet<usize>,
) -> Option<(usize, &'a LoweringTraceEvent)> {
    source_map
        .lowering_trace
        .iter()
        .enumerate()
        .filter(|(index, lowering_event)| {
            !used.contains(index)
                && lowering_event.core_event_id == event.id
                && lowering_event.function == program.target
                && lowering_event.kind == event.kind.as_str()
                && lowering_event.binding == event.binding
                && lowering_event.order == event.order
        })
        .min_by_key(|(_, lowering_event)| {
            lowering_event
                .kobo_span
                .start
                .abs_diff(event.source_span.start as u32)
        })
}

pub(super) fn generated_source_span(
    source_map: &KoboSourceMap,
    lowering_event: &LoweringTraceEvent,
) -> SourceSpan {
    SourceSpan {
        path: source_map.generated_file().to_owned(),
        line: lowering_event.rs_span.line,
        start: lowering_event.rs_span.column_start,
        end: lowering_event.rs_span.column_end,
        mapped: true,
        snippet: String::new(),
    }
}

pub(super) fn source_map_anchor_span_from_kobo_span(
    source_path: &str,
    source: &str,
    span: &KoboSpan,
) -> SourceSpan {
    let start = span.start as usize;
    let end = span.end.max(span.start + 1) as usize;
    let bounded_start = start.min(source.len());
    SourceSpan {
        path: source_path.to_owned(),
        line: one_based_line_for_offset(source, bounded_start),
        start,
        end,
        mapped: end > start,
        snippet: line_snippet(source, bounded_start),
    }
}
