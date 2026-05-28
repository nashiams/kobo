use kobo_ir::{ScenarioModeledBoundary, ScenarioOpKind, ScenarioProgram};

use crate::core::{ModeledBoundary, ScenarioEvent, ScenarioOptions};

pub(in super::super) fn boundary_label(boundary: &ScenarioModeledBoundary) -> &'static str {
    match boundary {
        ScenarioModeledBoundary::WardTime => "ward.time",
        ScenarioModeledBoundary::WardRandom => "ward.random",
        ScenarioModeledBoundary::WardTask => "ward.task",
        ScenarioModeledBoundary::WardTaskLocal => "ward.task.local",
    }
}

pub(in super::super) fn modeled_boundary_events(
    boundary: &ScenarioModeledBoundary,
    options: &ScenarioOptions,
) -> Vec<ScenarioEvent> {
    crate::scheduler::modeled_boundary_events(&core_boundary(boundary), options)
}

pub(super) fn first_modeled_boundary(program: &ScenarioProgram) -> Option<&'static str> {
    program.operations.iter().find_map(|operation| {
        if let ScenarioOpKind::ModeledEffect { boundary } = &operation.kind {
            Some(boundary_label(boundary))
        } else {
            None
        }
    })
}

pub(super) fn core_boundary(boundary: &ScenarioModeledBoundary) -> ModeledBoundary {
    match boundary {
        ScenarioModeledBoundary::WardTime => ModeledBoundary::WardTime,
        ScenarioModeledBoundary::WardRandom => ModeledBoundary::WardRandom,
        ScenarioModeledBoundary::WardTask => ModeledBoundary::WardTask,
        ScenarioModeledBoundary::WardTaskLocal => ModeledBoundary::WardTaskLocal,
    }
}
