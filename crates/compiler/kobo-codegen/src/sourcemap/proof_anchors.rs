use kobo_ir::{ScenarioCoreTerminatorKind, ScenarioOpKind, ScenarioProgram};

use super::entries::SourceMapEntry;
use super::lowering_trace::{
    core_event_id_for_operation, lowering_event_from_operation, proof_source_map_entry_id,
    template_by_binding, TraceEventSource,
};
use super::syntax_anchors::generated_proof_event_anchors;
use super::RsSpan;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GeneratedProofEventRole {
    Create,
    Discharge,
    Transfer,
    Move,
    Escape,
    ReturnFunction,
    ReturnTerminator,
    ErrorExit,
    Panic,
    Cancel,
    OpaqueBoundary,
}

#[derive(Clone, Debug)]
pub(super) struct GeneratedProofEventAnchor {
    pub(super) function: String,
    pub(super) kind: &'static str,
    pub(super) binding: Option<String>,
    pub(super) detail: Option<String>,
    pub(super) role: GeneratedProofEventRole,
    pub(super) rs_span: RsSpan,
}

pub(crate) fn add_proof_event_source_entries(
    entries: &mut Vec<SourceMapEntry>,
    rs_source: &str,
    programs: &[ScenarioProgram],
) {
    let generated_anchors = generated_proof_event_anchors(rs_source);
    let mut used_anchors = vec![false; generated_anchors.len()];
    for program in programs {
        let template_by_binding = template_by_binding(program);
        for (order, operation) in program.operations.iter().enumerate() {
            let Some(event) = lowering_event_from_operation(&operation.kind, &template_by_binding)
            else {
                continue;
            };
            let core_event_id =
                core_event_id_for_operation(&program.target, order, &operation.kind);
            let source_map_entry_id = proof_source_map_entry_id(&program.target, order);
            if has_core_event_anchor(entries, &source_map_entry_id, &core_event_id) {
                continue;
            }
            let Some(rs_span) = generated_anchor_for_operation(
                &generated_anchors,
                &mut used_anchors,
                program,
                &operation.kind,
                &event,
            ) else {
                continue;
            };
            entries.push(SourceMapEntry {
                id: source_map_entry_id,
                core_event_id: Some(core_event_id),
                binding_name: event
                    .binding
                    .clone()
                    .unwrap_or_else(|| event.kind.to_owned()),
                rs_span,
                kobo_span: operation.span,
                ownership_tier: "proof-event".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            });
        }
    }
}

fn has_core_event_anchor(
    entries: &[SourceMapEntry],
    source_map_entry_id: &str,
    core_event_id: &str,
) -> bool {
    entries.iter().any(|entry| {
        entry.id == source_map_entry_id && entry.core_event_id.as_deref() == Some(core_event_id)
    })
}

fn generated_anchor_for_operation(
    anchors: &[GeneratedProofEventAnchor],
    used_anchors: &mut [bool],
    program: &ScenarioProgram,
    operation: &ScenarioOpKind,
    event: &TraceEventSource<'_>,
) -> Option<RsSpan> {
    let role = generated_anchor_role(operation)?;
    let binding = event.binding.as_deref();
    let detail = generated_anchor_detail(operation);
    let anchor_matches = |index: usize, anchor: &GeneratedProofEventAnchor| {
        !used_anchors[index]
            && anchor.kind == event.kind
            && anchor.binding.as_deref() == binding
            && anchor.detail == detail
            && anchor.role == role
    };
    let reusable_anchor_matches = |anchor: &GeneratedProofEventAnchor| {
        anchor.kind == event.kind
            && anchor.binding.as_deref() == binding
            && anchor.detail == detail
            && anchor.role == role
    };
    let helper_anchor_matches = |index: usize, anchor: &GeneratedProofEventAnchor| {
        !used_anchors[index]
            && matches!(role, GeneratedProofEventRole::Discharge)
            && anchor.kind == event.kind
            && anchor.detail == detail
            && anchor.role == role
    };
    let reusable_helper_anchor_matches = |anchor: &GeneratedProofEventAnchor| {
        matches!(role, GeneratedProofEventRole::Discharge)
            && anchor.kind == event.kind
            && anchor.detail == detail
            && anchor.role == role
    };
    let (index, anchor) = anchors
        .iter()
        .enumerate()
        .find(|(index, anchor)| anchor.function == program.target && anchor_matches(*index, anchor))
        .or_else(|| {
            anchors.iter().enumerate().find(|(_, anchor)| {
                anchor.function != program.target && reusable_anchor_matches(anchor)
            })
        })
        .or_else(|| {
            anchors.iter().enumerate().find(|(_, anchor)| {
                anchor.function != program.target && reusable_helper_anchor_matches(anchor)
            })
        })
        .or_else(|| {
            anchors
                .iter()
                .enumerate()
                .find(|(index, anchor)| helper_anchor_matches(*index, anchor))
        })?;
    if anchor.function == program.target {
        used_anchors[index] = true;
    }
    Some(anchor.rs_span.clone())
}

fn generated_anchor_detail(operation: &ScenarioOpKind) -> Option<String> {
    match operation {
        ScenarioOpKind::Discharge { action, .. } => Some(discharge_anchor_detail(action)),
        ScenarioOpKind::Transfer { callee, .. } => Some(callee.clone()),
        ScenarioOpKind::ExternalBoundary {
            call_path,
            crate_name,
            ..
        } => Some(
            call_path
                .as_deref()
                .and_then(last_path_segment_text)
                .unwrap_or(crate_name)
                .to_owned(),
        ),
        ScenarioOpKind::CoreTerminator {
            kind: ScenarioCoreTerminatorKind::OpaqueBoundary,
            ..
        } => None,
        _ => None,
    }
}

fn discharge_anchor_detail(action: &str) -> String {
    if action.starts_with("escape:") {
        return "escape".to_owned();
    }
    if action.starts_with("suppressed:") {
        return "suppressed".to_owned();
    }
    action.to_owned()
}

fn last_path_segment_text(path: &str) -> Option<&str> {
    path.rsplit("::")
        .next()
        .filter(|segment| !segment.is_empty())
}

fn generated_anchor_role(operation: &ScenarioOpKind) -> Option<GeneratedProofEventRole> {
    match operation {
        ScenarioOpKind::CreateObligation { .. } => Some(GeneratedProofEventRole::Create),
        ScenarioOpKind::Discharge { .. } => Some(GeneratedProofEventRole::Discharge),
        ScenarioOpKind::Transfer { .. } => Some(GeneratedProofEventRole::Transfer),
        ScenarioOpKind::MoveBinding { .. } => Some(GeneratedProofEventRole::Move),
        ScenarioOpKind::ExternalBoundary { .. } => Some(GeneratedProofEventRole::Escape),
        ScenarioOpKind::Return => Some(GeneratedProofEventRole::ReturnFunction),
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(core_terminator_anchor_role(kind)),
        _ => None,
    }
}

fn core_terminator_anchor_role(kind: &ScenarioCoreTerminatorKind) -> GeneratedProofEventRole {
    match kind {
        ScenarioCoreTerminatorKind::Return => GeneratedProofEventRole::ReturnTerminator,
        ScenarioCoreTerminatorKind::ErrorExit => GeneratedProofEventRole::ErrorExit,
        ScenarioCoreTerminatorKind::Panic => GeneratedProofEventRole::Panic,
        ScenarioCoreTerminatorKind::Await => GeneratedProofEventRole::Cancel,
        ScenarioCoreTerminatorKind::OpaqueBoundary => GeneratedProofEventRole::OpaqueBoundary,
    }
}
