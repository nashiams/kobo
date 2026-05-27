use super::{
    env_states, function_obligation_replay, source_span_for_binding, source_span_from_kobo,
    stable_hash, AsyncModelEvidence, CancelEdgeEvidence, CoreFunction, CoreTerminatorKind,
    FutureStateLocalEvidence, FutureStateObligationEvidence, ObligationState, ObligationStatus,
    ScenarioOpKind, ScenarioProgram, SelectPathEvidence, SpawnedTaskObligationEvidence,
    SuspensionStateEvidence, TimeoutCancelEdgeEvidence,
};

mod live_locals;

use live_locals::parsed_live_locals_by_await;

pub(super) fn async_model_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    functions: &[CoreFunction],
) -> AsyncModelEvidence {
    let live_locals_by_await = parsed_live_locals_by_await(source, &program.target);
    let mut model = AsyncModelEvidence::default();

    for function in functions {
        let replay = function_obligation_replay(function);
        let mut await_index = 0usize;
        for block in &function.blocks {
            let block_exit_env = replay
                .block_exit_envs
                .get(&block.id)
                .cloned()
                .unwrap_or_default();
            for terminator in &block.terminators {
                match terminator.kind {
                    CoreTerminatorKind::Await => {
                        let suspension_id =
                            format!("{}:{}:{}", function.name, block.id, terminator.id);
                        let resume_edge = terminator
                            .edges
                            .iter()
                            .find(|edge| edge.as_str() == "await_resume")
                            .cloned()
                            .unwrap_or_else(|| "await_resume".to_owned());
                        let cancel_edge = terminator
                            .edges
                            .iter()
                            .find(|edge| edge.as_str() == "await_cancel")
                            .cloned()
                            .unwrap_or_else(|| "await_cancel".to_owned());
                        let source_span =
                            source_span_from_kobo(source_path, source, terminator.source_span);
                        model.suspension_states.push(SuspensionStateEvidence {
                            id: suspension_id.clone(),
                            function: function.name.clone(),
                            block: block.id.clone(),
                            terminator_kind: "await".to_owned(),
                            boundary: terminator.boundary.clone(),
                            resume_edge,
                            cancel_edge: cancel_edge.clone(),
                            source_span: source_span.clone(),
                        });
                        model.cancel_edges.push(CancelEdgeEvidence {
                            id: format!("{suspension_id}:future-drop"),
                            function: function.name.clone(),
                            from: block.id.clone(),
                            to: cancel_edge.clone(),
                            reason: "future_drop".to_owned(),
                            source_span: source_span.clone(),
                        });
                        let live_locals = live_locals_by_await
                            .get(await_index)
                            .cloned()
                            .unwrap_or_default();
                        await_index += 1;
                        for local in &live_locals {
                            model.future_state_locals.push(FutureStateLocalEvidence {
                                binding: local.clone(),
                                suspension_state: suspension_id.clone(),
                                source_span: source_span_for_binding(source_path, source, local),
                            });
                        }
                        for state in env_states(&block_exit_env)
                            .into_iter()
                            .filter(|state| state.state == ObligationStatus::Owned)
                        {
                            model
                                .future_state_obligations
                                .push(FutureStateObligationEvidence {
                                    binding: state.binding,
                                    state: state.state,
                                    suspension_state: suspension_id.clone(),
                                    source_span: source_span.clone(),
                                });
                        }
                        if terminator.boundary.as_deref() == Some("tokio::time::timeout") {
                            model.timeout_cancel_edges.push(TimeoutCancelEdgeEvidence {
                                id: format!("{suspension_id}:timeout"),
                                function: function.name.clone(),
                                suspension_state: suspension_id,
                                source: "tokio::time::timeout".to_owned(),
                                cancel_edge,
                                source_span,
                            });
                        }
                    }
                    CoreTerminatorKind::Branch => {
                        let source_span =
                            source_span_from_kobo(source_path, source, terminator.source_span);
                        let cancelled_obligations = env_states(&block_exit_env)
                            .into_iter()
                            .filter(|state| state.state == ObligationStatus::Owned)
                            .collect::<Vec<_>>();
                        for branch_target in terminator
                            .edges
                            .iter()
                            .filter_map(|edge| edge.strip_prefix("goto:"))
                        {
                            let obligation_results = replay
                                .block_exit_envs
                                .get(branch_target)
                                .map(env_states)
                                .unwrap_or_default();
                            for path_kind in ["winner", "loser_cancel"] {
                                let cancelled_obligations = (path_kind == "loser_cancel")
                                    .then(|| cancelled_obligations.clone())
                                    .unwrap_or_default();
                                model.select_paths.push(SelectPathEvidence {
                                    id: format!(
                                        "{}:{}:{}:{branch_target}:{path_kind}",
                                        function.name, block.id, terminator.id
                                    ),
                                    function: function.name.clone(),
                                    branch_block: block.id.clone(),
                                    branch_target: branch_target.to_owned(),
                                    path_kind: path_kind.to_owned(),
                                    obligation_results: obligation_results.clone(),
                                    cancelled_obligations: cancelled_obligations.clone(),
                                    obligation_result_hash: canonical_select_result_hash(
                                        branch_target,
                                        path_kind,
                                        &obligation_results,
                                        &cancelled_obligations,
                                    ),
                                    source_span: source_span.clone(),
                                });
                            }
                        }
                    }
                    CoreTerminatorKind::Goto
                    | CoreTerminatorKind::Return
                    | CoreTerminatorKind::ErrorExit
                    | CoreTerminatorKind::Panic
                    | CoreTerminatorKind::OpaqueBoundary => {}
                }
            }
        }
    }

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

    model
}

pub(super) fn canonical_select_result_hash(
    branch_target: &str,
    path_kind: &str,
    results: &[ObligationState],
    cancelled_obligations: &[ObligationState],
) -> String {
    let mut states = results
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    states.sort_unstable();
    let mut cancelled = cancelled_obligations
        .iter()
        .map(|state| format!("{}:{}", state.binding, state.state.as_str()))
        .collect::<Vec<_>>();
    cancelled.sort_unstable();
    stable_hash(&format!(
        "select-result:{branch_target}:{path_kind}:{states:?}:cancelled:{cancelled:?}"
    ))
}
