use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{LoweredScenario, ModeledBoundary, ScenarioCoverage, ScenarioOperation};

pub const MODEL_VERSION: &str = "v0.9-driver-kir-scenario-full-depth-2";

pub fn lower_from_program(program: &ScenarioProgram, fallback_profile: &str) -> LoweredScenario {
    let operations = program
        .operations
        .iter()
        .filter_map(|operation| {
            let span_start = operation.span.start as usize;
            let span_end = operation.span.end.max(operation.span.start + 1) as usize;
            match &operation.kind {
                ScenarioOpKind::CreateObligation {
                    binding,
                    type_name,
                    actions,
                } => Some(ScenarioOperation::CreateObligation {
                    binding: binding.clone(),
                    type_name: type_name.clone(),
                    actions: actions.clone(),
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::Discharge { binding, action } => {
                    Some(ScenarioOperation::Discharge {
                        binding: binding.clone(),
                        action: action.clone(),
                    })
                }
                ScenarioOpKind::Transfer { binding, callee } => Some(ScenarioOperation::Transfer {
                    binding: binding.clone(),
                    callee: callee.clone(),
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::MoveBinding { binding } => Some(ScenarioOperation::MoveBinding {
                    binding: binding.clone(),
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::ModeledEffect { boundary } => {
                    Some(ScenarioOperation::ModeledEffect {
                        boundary: modeled_boundary(boundary),
                        span_start,
                        span_end,
                    })
                }
                ScenarioOpKind::StorageEvent { action } => Some(ScenarioOperation::StorageEvent {
                    action: action.clone(),
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::NetworkEvent { action } => Some(ScenarioOperation::NetworkEvent {
                    action: action.clone(),
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::RawNondeterminism { operation } => {
                    Some(ScenarioOperation::RawNondeterminism {
                        operation: operation.clone(),
                        span_start,
                        span_end,
                    })
                }
                ScenarioOpKind::UncontrolledEffect { operation } => {
                    Some(ScenarioOperation::UncontrolledEffect {
                        operation: operation.clone(),
                        span_start,
                        span_end,
                    })
                }
                ScenarioOpKind::ExternalBoundary { crate_name } => {
                    Some(ScenarioOperation::ExternalBoundary {
                        crate_name: crate_name.clone(),
                        span_start,
                        span_end,
                    })
                }
                ScenarioOpKind::Loop => Some(ScenarioOperation::Loop {
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::Return => None,
            }
        })
        .collect();

    LoweredScenario {
        profile: fallback_profile.to_owned(),
        operations,
        coverage: ScenarioCoverage {
            unsupported_constructs: program.coverage.unsupported_constructs.clone(),
            reason: None,
        },
    }
}

fn modeled_boundary(boundary: &ScenarioModeledBoundary) -> ModeledBoundary {
    match boundary {
        ScenarioModeledBoundary::WardTime => ModeledBoundary::WardTime,
        ScenarioModeledBoundary::WardRandom => ModeledBoundary::WardRandom,
        ScenarioModeledBoundary::WardTask => ModeledBoundary::WardTask,
    }
}
