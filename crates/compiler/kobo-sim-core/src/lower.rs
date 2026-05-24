use kobo_ir::{ScenarioBoundaryPolicy, ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{
    BoundaryPolicyChoice, LoweredScenario, ModeledBoundary, ScenarioCoverage, ScenarioOperation,
};

pub const MODEL_SCHEMA: &str = "driver-kir-generated-user-rust-harness-call-shape";
pub const MODEL_SCHEMA_VERSION: u64 = 1;

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
                    ..
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
                ScenarioOpKind::Transfer {
                    binding, callee, ..
                } => Some(ScenarioOperation::Transfer {
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
                ScenarioOpKind::BranchUnresolved { binding } => {
                    Some(ScenarioOperation::BranchUnresolved {
                        binding: binding.clone(),
                        span_start,
                        span_end,
                    })
                }
                ScenarioOpKind::UnsupportedContainer {
                    binding,
                    type_name,
                    container,
                } => Some(ScenarioOperation::UnsupportedContainer {
                    binding: binding.clone(),
                    type_name: type_name.clone(),
                    container: container.clone(),
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
                ScenarioOpKind::Select { branch_count } => Some(ScenarioOperation::Select {
                    branch_count: *branch_count,
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
                ScenarioOpKind::ExternalBoundary {
                    crate_name,
                    call_path,
                    call_arguments,
                    return_type,
                    call_shape,
                    policy,
                    reason,
                } => Some(ScenarioOperation::ExternalBoundary {
                    crate_name: crate_name.clone(),
                    call_path: call_path.clone(),
                    call_arguments: call_arguments.clone(),
                    return_type: return_type.clone(),
                    call_shape: call_shape.clone(),
                    policy: boundary_policy(policy),
                    reason: reason.clone(),
                    span_start,
                    span_end,
                }),
                ScenarioOpKind::Loop | ScenarioOpKind::LoopBackEdge => {
                    Some(ScenarioOperation::Loop {
                        span_start,
                        span_end,
                    })
                }
                ScenarioOpKind::LoopStart
                | ScenarioOpKind::LoopContinue
                | ScenarioOpKind::LoopBreak => None,
                ScenarioOpKind::CoreTerminator { .. } => None,
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

fn boundary_policy(policy: &ScenarioBoundaryPolicy) -> BoundaryPolicyChoice {
    match policy {
        ScenarioBoundaryPolicy::Typed => BoundaryPolicyChoice::Typed,
        ScenarioBoundaryPolicy::Model => BoundaryPolicyChoice::Model,
        ScenarioBoundaryPolicy::Record => BoundaryPolicyChoice::Record,
        ScenarioBoundaryPolicy::Activity => BoundaryPolicyChoice::Activity,
        ScenarioBoundaryPolicy::Stub => BoundaryPolicyChoice::Stub,
        ScenarioBoundaryPolicy::Outside => BoundaryPolicyChoice::Outside,
        ScenarioBoundaryPolicy::Opaque => BoundaryPolicyChoice::Opaque,
        ScenarioBoundaryPolicy::Debt => BoundaryPolicyChoice::Debt,
        ScenarioBoundaryPolicy::Unselected => BoundaryPolicyChoice::Unselected,
    }
}

fn modeled_boundary(boundary: &ScenarioModeledBoundary) -> ModeledBoundary {
    match boundary {
        ScenarioModeledBoundary::WardTime => ModeledBoundary::WardTime,
        ScenarioModeledBoundary::WardRandom => ModeledBoundary::WardRandom,
        ScenarioModeledBoundary::WardTask => ModeledBoundary::WardTask,
        ScenarioModeledBoundary::WardTaskLocal => ModeledBoundary::WardTaskLocal,
    }
}
