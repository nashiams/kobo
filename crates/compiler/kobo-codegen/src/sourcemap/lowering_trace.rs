use std::collections::BTreeMap;

use kobo_ir::{
    KoboSpan, ScenarioCoreTerminatorKind, ScenarioLifecycleTemplate,
    ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
use serde::{Deserialize, Serialize};

use super::entries::{KoboSourceMap, SourceMapEntry};
use super::RsSpan;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoweringTraceEvent {
    pub id: String,
    pub core_event_id: String,
    pub function: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    pub order: u64,
    pub source_map_entry_id: String,
    pub rs_span: RsSpan,
    pub kobo_span: KoboSpan,
    pub lowering_phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
}

pub(crate) fn build_lowering_trace(
    programs: &[ScenarioProgram],
    source_map: &KoboSourceMap,
) -> Vec<LoweringTraceEvent> {
    programs
        .iter()
        .flat_map(|program| {
            let template_by_binding = template_by_binding(program);
            let has_obligation_trace_events = has_obligation_trace_events(program);
            program
                .operations
                .iter()
                .enumerate()
                .filter_map(move |(order, operation)| {
                    if !has_obligation_trace_events
                        && matches!(operation.kind, ScenarioOpKind::Return)
                    {
                        return None;
                    }
                    let event =
                        lowering_event_from_operation(&operation.kind, &template_by_binding)?;
                    let core_event_id =
                        core_event_id_for_operation(&program.target, order, &operation.kind);
                    let source_map_entry_id = proof_source_map_entry_id(&program.target, order);
                    let anchor =
                        anchor_for_trace_event(source_map, &source_map_entry_id, &core_event_id)?;
                    Some(LoweringTraceEvent {
                        id: format!("lowering-{}-{order}", program.target),
                        core_event_id,
                        function: program.target.clone(),
                        kind: event.kind.to_owned(),
                        binding: event.binding,
                        order: order as u64,
                        source_map_entry_id: anchor.id.clone(),
                        rs_span: anchor.rs_span.clone(),
                        kobo_span: anchor.kobo_span,
                        lowering_phase: "kobo-codegen".to_owned(),
                        template_id: event.template.map(|template| template.id.clone()),
                        template_version: event.template.map(template_version),
                    })
                })
        })
        .collect()
}

fn has_obligation_trace_events(program: &ScenarioProgram) -> bool {
    program.operations.iter().any(|operation| {
        matches!(
            operation.kind,
            ScenarioOpKind::CreateObligation { .. }
                | ScenarioOpKind::Discharge { .. }
                | ScenarioOpKind::Transfer { .. }
                | ScenarioOpKind::MoveBinding { .. }
                | ScenarioOpKind::ExternalBoundary { .. }
        )
    })
}

pub(super) fn proof_source_map_entry_id(function: &str, order: usize) -> String {
    format!("proof-map-{function}-{order}")
}

pub(super) fn core_event_id_for_operation(
    function: &str,
    order: usize,
    kind: &ScenarioOpKind,
) -> String {
    if matches!(
        kind,
        ScenarioOpKind::CreateObligation { .. }
            | ScenarioOpKind::Discharge { .. }
            | ScenarioOpKind::Transfer { .. }
            | ScenarioOpKind::MoveBinding { .. }
            | ScenarioOpKind::ExternalBoundary { .. }
    ) {
        return format!("core-{function}-stmt-{order}");
    }
    format!("core-{function}-term-{order}")
}

pub(super) struct TraceEventSource<'a> {
    pub(super) kind: &'static str,
    pub(super) binding: Option<String>,
    pub(super) template: Option<&'a ScenarioLifecycleTemplate>,
}

pub(super) fn lowering_event_from_operation<'a>(
    kind: &'a ScenarioOpKind,
    template_by_binding: &BTreeMap<&'a str, &'a ScenarioLifecycleTemplate>,
) -> Option<TraceEventSource<'a>> {
    match kind {
        ScenarioOpKind::CreateObligation {
            binding, template, ..
        } => Some(TraceEventSource {
            kind: "create",
            binding: Some(binding.clone()),
            template: template
                .as_ref()
                .or_else(|| template_by_binding.get(binding.as_str()).copied()),
        }),
        ScenarioOpKind::Discharge { binding, .. } => Some(binding_event_source(
            "discharge",
            binding,
            template_by_binding,
        )),
        ScenarioOpKind::Transfer { binding, .. } => Some(binding_event_source(
            "transfer",
            binding,
            template_by_binding,
        )),
        ScenarioOpKind::MoveBinding { binding } => {
            Some(binding_event_source("move", binding, template_by_binding))
        }
        ScenarioOpKind::ExternalBoundary { .. } => Some(TraceEventSource {
            kind: "escape",
            binding: None,
            template: None,
        }),
        ScenarioOpKind::Return => Some(TraceEventSource {
            kind: "return",
            binding: None,
            template: None,
        }),
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(TraceEventSource {
            kind: core_terminator_trace_kind(kind),
            binding: None,
            template: None,
        }),
        _ => None,
    }
}

fn binding_event_source<'a>(
    kind: &'static str,
    binding: &'a str,
    template_by_binding: &BTreeMap<&'a str, &'a ScenarioLifecycleTemplate>,
) -> TraceEventSource<'a> {
    TraceEventSource {
        kind,
        binding: Some(binding.to_owned()),
        template: template_by_binding.get(binding).copied(),
    }
}

pub(super) fn template_by_binding(
    program: &ScenarioProgram,
) -> BTreeMap<&str, &ScenarioLifecycleTemplate> {
    program
        .operations
        .iter()
        .filter_map(|operation| match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding,
                template: Some(template),
                ..
            } => Some((binding.as_str(), template)),
            _ => None,
        })
        .collect()
}

fn core_terminator_trace_kind(kind: &ScenarioCoreTerminatorKind) -> &'static str {
    match kind {
        ScenarioCoreTerminatorKind::Return => "return",
        ScenarioCoreTerminatorKind::ErrorExit => "error_exit",
        ScenarioCoreTerminatorKind::Panic => "panic",
        ScenarioCoreTerminatorKind::Await => "cancel",
        ScenarioCoreTerminatorKind::OpaqueBoundary => "opaque_boundary",
    }
}

fn anchor_for_trace_event<'a>(
    source_map: &'a KoboSourceMap,
    source_map_entry_id: &str,
    core_event_id: &str,
) -> Option<&'a SourceMapEntry> {
    source_map.x_kobo_mappings.iter().find(|entry| {
        entry.id == source_map_entry_id && entry.core_event_id.as_deref() == Some(core_event_id)
    })
}

fn template_version(template: &ScenarioLifecycleTemplate) -> String {
    if matches!(template.source, ScenarioLifecycleTemplateSource::Inference) {
        "0.1".to_owned()
    } else {
        template.schema_version.to_string()
    }
}
