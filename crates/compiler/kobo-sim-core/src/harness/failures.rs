mod boundaries;

use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{ScenarioEvent, ScenarioOptions};
use crate::network::NetworkModel;
use crate::storage::StorageModel;

pub(super) use boundaries::{boundary_label, modeled_boundary_events};
use boundaries::{core_boundary, first_modeled_boundary};

struct HarnessObligation {
    binding: String,
    actions: Vec<String>,
    is_discharged: bool,
    declaration_span: (usize, usize),
}

impl HarnessObligation {
    fn new(binding: String, actions: Vec<String>, declaration_span: (usize, usize)) -> Self {
        Self {
            binding,
            actions,
            is_discharged: false,
            declaration_span,
        }
    }
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
        let selected_failure = apply_operation_effects(
            operation,
            options,
            &mut obligations,
            &mut storage,
            &mut network,
            &mut events,
        );
        if failure_events.is_none() {
            failure_events = selected_failure;
        }
    }
    failure_events = failure_events
        .or_else(|| cancel_failure_events(program, &obligations, options))
        .or_else(|| liveness_failure_events(&obligations));
    if let Some(failure) = failure_events {
        events.extend(failure);
    }
    events.extend(crate::core::scheduler_events(options));
    events
}

fn apply_operation_effects(
    operation: &kobo_ir::ScenarioOp,
    options: &ScenarioOptions,
    obligations: &mut Vec<HarnessObligation>,
    storage: &mut StorageModel,
    network: &mut NetworkModel,
    events: &mut Vec<ScenarioEvent>,
) -> Option<Vec<ScenarioEvent>> {
    let span = (
        operation.span.start as usize,
        operation.span.end.max(operation.span.start + 1) as usize,
    );
    match &operation.kind {
        ScenarioOpKind::CreateObligation {
            binding, actions, ..
        } => {
            obligations.push(HarnessObligation::new(
                binding.clone(),
                actions.clone(),
                span,
            ));
            None
        }
        ScenarioOpKind::Discharge { binding, action } => {
            storage.note_obligation_action(action);
            discharge_obligation(obligations, binding, action);
            None
        }
        ScenarioOpKind::Transfer {
            binding, callee, ..
        } => {
            events.push(transfer_event(binding, callee));
            None
        }
        ScenarioOpKind::UnsupportedContainer {
            binding,
            type_name,
            container,
        } => Some(unsupported_container_failure(binding, type_name, container)),
        ScenarioOpKind::BranchUnresolved { binding } => Some(branch_unresolved_failure(binding)),
        ScenarioOpKind::ModeledEffect { boundary } => {
            scheduler_failure(boundary, options, obligations, span)
        }
        ScenarioOpKind::StorageEvent { action } => storage
            .apply_action(action, options.seed, span)
            .failure
            .map(|failure| failure.events),
        ScenarioOpKind::NetworkEvent { action } => network
            .apply_action(action, options.seed, span)
            .failure
            .map(|failure| failure.events),
        _ => None,
    }
}

fn discharge_obligation(obligations: &mut [HarnessObligation], binding: &str, action: &str) {
    let Some(obligation) = obligations
        .iter_mut()
        .rev()
        .find(|obligation| obligation.binding == binding && !obligation.is_discharged)
    else {
        return;
    };
    if obligation
        .actions
        .iter()
        .any(|candidate| candidate == action)
    {
        obligation.is_discharged = true;
    }
}

fn transfer_event(binding: &str, callee: &str) -> ScenarioEvent {
    ScenarioEvent {
        kind: "obligation-transfer".to_owned(),
        label: Some(format!("{binding}->{callee}")),
        value: None,
        io: None,
    }
}

fn unsupported_container_failure(
    binding: &str,
    type_name: &str,
    container: &str,
) -> Vec<ScenarioEvent> {
    vec![ScenarioEvent {
        kind: "unsupported-container".to_owned(),
        label: Some(format!("{binding}:{container}:{type_name}")),
        value: None,
        io: None,
    }]
}

fn branch_unresolved_failure(binding: &str) -> Vec<ScenarioEvent> {
    vec![ScenarioEvent {
        kind: "branch-unresolved".to_owned(),
        label: Some(binding.to_owned()),
        value: None,
        io: None,
    }]
}

fn scheduler_failure(
    boundary: &ScenarioModeledBoundary,
    options: &ScenarioOptions,
    obligations: &[HarnessObligation],
    span: (usize, usize),
) -> Option<Vec<ScenarioEvent>> {
    let active_obligation = obligations
        .iter()
        .rev()
        .find(|obligation| !obligation.is_discharged)
        .map(|obligation| (obligation.binding.as_str(), obligation.declaration_span));
    crate::scheduler::schedule_failure(&core_boundary(boundary), options, active_obligation, span)
        .map(|failure| failure.events)
}

fn cancel_failure_events(
    program: &ScenarioProgram,
    obligations: &[HarnessObligation],
    options: &ScenarioOptions,
) -> Option<Vec<ScenarioEvent>> {
    if !has_cancel_injection(options) {
        return None;
    }
    obligations
        .iter()
        .find(|obligation| !obligation.is_discharged)
        .map(|obligation| obligation.binding.clone())
        .or_else(|| first_modeled_boundary(program).map(str::to_owned))
        .map(|label| {
            vec![ScenarioEvent {
                kind: "failure-injection-cancel".to_owned(),
                label: Some(label),
                value: None,
                io: None,
            }]
        })
}

fn liveness_failure_events(obligations: &[HarnessObligation]) -> Option<Vec<ScenarioEvent>> {
    obligations
        .iter()
        .find(|obligation| !obligation.is_discharged)
        .map(|obligation| {
            vec![ScenarioEvent {
                kind: "liveness-token-drop".to_owned(),
                label: Some(obligation.binding.clone()),
                value: None,
                io: None,
            }]
        })
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
