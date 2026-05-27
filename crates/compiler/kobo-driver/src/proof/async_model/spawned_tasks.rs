use kobo_ir::{ScenarioOpKind, ScenarioProgram};
use kobo_proof::{AsyncModelEvidence, SpawnedTaskObligationEvidence};

use super::super::source_spans::source_span_from_kobo;

pub(super) fn collect_spawned_task_obligations(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    model: &mut AsyncModelEvidence,
) {
    for operation in &program.operations {
        let ScenarioOpKind::CreateObligation {
            binding,
            type_name,
            actions,
            template,
        } = &operation.kind
        else {
            continue;
        };
        let is_spawned_task = type_name == "SpawnedTask"
            || template
                .as_ref()
                .is_some_and(|template| template.kind == "spawned_task");
        if is_spawned_task {
            model
                .spawned_task_obligations
                .push(SpawnedTaskObligationEvidence {
                    binding: binding.clone(),
                    required_resolution: actions.clone(),
                    source_span: source_span_from_kobo(source_path, source, operation.span),
                });
        }
    }
}
