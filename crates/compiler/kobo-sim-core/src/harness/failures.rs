use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{ScenarioEvent, ScenarioOptions};
use crate::network::NetworkModel;
use crate::storage::StorageModel;

struct HarnessObligation {
    binding: String,
    actions: Vec<String>,
    is_discharged: bool,
    declaration_span: (usize, usize),
}

pub(super) fn terminal_failure_events(
    program: &ScenarioProgram,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    let mut events = Vec::new();
    let mut obligations = Vec::new();
    let mut storage = StorageModel::default();
    let mut network = NetworkModel::default();
    let mut failure_events = None;
    for operation in &program.operations {
        let span = (
            operation.span.start as usize,
            operation.span.end.max(operation.span.start + 1) as usize,
        );
        match &operation.kind {
            ScenarioOpKind::CreateObligation {
                binding, actions, ..
            } => obligations.push(HarnessObligation {
                binding: binding.clone(),
                actions: actions.clone(),
                is_discharged: false,
                declaration_span: span,
            }),
            ScenarioOpKind::Discharge { binding, action } => {
                storage.note_obligation_action(action);
                if let Some(obligation) = obligations
                    .iter_mut()
                    .rev()
                    .find(|obligation| obligation.binding == *binding && !obligation.is_discharged)
                {
                    if obligation
                        .actions
                        .iter()
                        .any(|candidate| candidate == action)
                    {
                        obligation.is_discharged = true;
                    }
                }
            }
            ScenarioOpKind::Transfer {
                binding, callee, ..
            } => events.push(ScenarioEvent {
                kind: "obligation-transfer".to_owned(),
                label: Some(format!("{binding}->{callee}")),
                value: None,
                io: None,
            }),
            ScenarioOpKind::UnsupportedContainer {
                binding,
                type_name,
                container,
            } => {
                if failure_events.is_none() {
                    failure_events = Some(vec![ScenarioEvent {
                        kind: "unsupported-container".to_owned(),
                        label: Some(format!("{binding}:{container}:{type_name}")),
                        value: None,
                        io: None,
                    }]);
                }
            }
            ScenarioOpKind::BranchUnresolved { binding } => {
                if failure_events.is_none() {
                    failure_events = Some(vec![ScenarioEvent {
                        kind: "branch-unresolved".to_owned(),
                        label: Some(binding.clone()),
                        value: None,
                        io: None,
                    }]);
                }
            }
            ScenarioOpKind::ModeledEffect { boundary } => {
                if failure_events.is_none() {
                    let active_obligation = obligations
                        .iter()
                        .rev()
                        .find(|obligation| !obligation.is_discharged)
                        .map(|obligation| {
                            (obligation.binding.as_str(), obligation.declaration_span)
                        });
                    if let Some(failure) = crate::scheduler::schedule_failure(
                        &core_boundary(boundary),
                        options,
                        active_obligation,
                        span,
                    ) {
                        failure_events = Some(failure.events);
                    }
                }
            }
            ScenarioOpKind::StorageEvent { action } => {
                if failure_events.is_none() {
                    if let Some(failure) = storage.apply_action(action, options.seed, span).failure
                    {
                        failure_events = Some(failure.events);
                    }
                }
            }
            ScenarioOpKind::NetworkEvent { action } => {
                if failure_events.is_none() {
                    if let Some(failure) = network.apply_action(action, options.seed, span).failure
                    {
                        failure_events = Some(failure.events);
                    }
                }
            }
            ScenarioOpKind::Select { .. } => {}
            ScenarioOpKind::RawNondeterminism { .. }
            | ScenarioOpKind::UncontrolledEffect { .. }
            | ScenarioOpKind::ExternalBoundary { .. }
            | ScenarioOpKind::CoreTerminator { .. }
            | ScenarioOpKind::LoopStart { .. }
            | ScenarioOpKind::LoopBackEdge { .. }
            | ScenarioOpKind::LoopContinue { .. }
            | ScenarioOpKind::LoopBreak { .. }
            | ScenarioOpKind::Loop => {}
            ScenarioOpKind::MoveBinding { .. } | ScenarioOpKind::Return => {}
        }
    }
    if failure_events.is_none() && has_cancel_injection(options) {
        if let Some(obligation) = obligations
            .iter()
            .find(|obligation| !obligation.is_discharged)
        {
            failure_events = Some(vec![ScenarioEvent {
                kind: "failure-injection-cancel".to_owned(),
                label: Some(obligation.binding.clone()),
                value: None,
                io: None,
            }]);
        }
        if failure_events.is_none() {
            if let Some(boundary) = first_modeled_boundary(program) {
                failure_events = Some(vec![ScenarioEvent {
                    kind: "failure-injection-cancel".to_owned(),
                    label: Some(boundary.to_owned()),
                    value: None,
                    io: None,
                }]);
            }
        }
    }

    if failure_events.is_none() {
        failure_events = obligations
            .iter()
            .find(|obligation| !obligation.is_discharged)
            .map(|obligation| {
                vec![ScenarioEvent {
                    kind: "liveness-token-drop".to_owned(),
                    label: Some(obligation.binding.clone()),
                    value: None,
                    io: None,
                }]
            });
    }
    if let Some(failure) = failure_events {
        events.extend(failure);
    }
    events.extend(crate::core::scheduler_events(options));
    events
}

pub(super) fn boundary_label(boundary: &ScenarioModeledBoundary) -> &'static str {
    match boundary {
        ScenarioModeledBoundary::WardTime => "ward.time",
        ScenarioModeledBoundary::WardRandom => "ward.random",
        ScenarioModeledBoundary::WardTask => "ward.task",
        ScenarioModeledBoundary::WardTaskLocal => "ward.task.local",
    }
}

pub(super) fn modeled_boundary_events(
    boundary: &ScenarioModeledBoundary,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    crate::scheduler::modeled_boundary_events(&core_boundary(boundary), options)
}

fn has_cancel_injection(options: &ScenarioOptions) -> bool {
    options
        .inject
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .any(|hook| hook == "cancel")
}

fn first_modeled_boundary(program: &ScenarioProgram) -> Option<&'static str> {
    program.operations.iter().find_map(|operation| {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            Some(boundary_label(boundary))
        } else {
            None
        }
    })
}

fn core_boundary(boundary: &ScenarioModeledBoundary) -> crate::core::ModeledBoundary {
    match boundary {
        ScenarioModeledBoundary::WardTime => crate::core::ModeledBoundary::WardTime,
        ScenarioModeledBoundary::WardRandom => crate::core::ModeledBoundary::WardRandom,
        ScenarioModeledBoundary::WardTask => crate::core::ModeledBoundary::WardTask,
        ScenarioModeledBoundary::WardTaskLocal => crate::core::ModeledBoundary::WardTaskLocal,
    }
}
