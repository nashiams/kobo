use std::collections::BTreeMap;

use kobo_ir::{CoreBlock, CoreFunction, CoreTerminator, CoreTerminatorKind, ScenarioProgram};
use kobo_proof::{AsyncModelEvidence, ObligationStatus};

use super::obligations::{function_obligation_replay, FunctionObligationReplay};

mod live_locals;
mod select_paths;
mod spawned_tasks;
mod suspensions;

use live_locals::parsed_live_locals_by_await;
use select_paths::record_select_paths;
use spawned_tasks::collect_spawned_task_obligations;
use suspensions::record_await_suspension;

pub(super) fn async_model_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    functions: &[CoreFunction],
) -> AsyncModelEvidence {
    let live_locals_by_await = parsed_live_locals_by_await(source, &program.target);
    let mut model = AsyncModelEvidence::default();

    for function in functions {
        collect_function_async_evidence(
            source_path,
            source,
            function,
            &live_locals_by_await,
            &mut model,
        );
    }

    collect_spawned_task_obligations(source_path, source, program, &mut model);
    model
}

fn collect_function_async_evidence(
    source_path: &str,
    source: &str,
    function: &CoreFunction,
    live_locals_by_await: &[Vec<String>],
    model: &mut AsyncModelEvidence,
) {
    let replay = function_obligation_replay(function);
    let mut await_index = 0usize;
    for block in &function.blocks {
        let block_exit_env = replay
            .block_exit_envs
            .get(&block.id)
            .cloned()
            .unwrap_or_default();
        for terminator in &block.terminators {
            record_terminator_async_evidence(
                source_path,
                source,
                function,
                block,
                terminator,
                &block_exit_env,
                &replay,
                live_locals_by_await,
                &mut await_index,
                model,
            );
        }
    }
}

fn record_terminator_async_evidence(
    source_path: &str,
    source: &str,
    function: &CoreFunction,
    block: &CoreBlock,
    terminator: &CoreTerminator,
    block_exit_env: &BTreeMap<String, ObligationStatus>,
    replay: &FunctionObligationReplay,
    live_locals_by_await: &[Vec<String>],
    await_index: &mut usize,
    model: &mut AsyncModelEvidence,
) {
    match terminator.kind {
        CoreTerminatorKind::Await => record_await_suspension(
            source_path,
            source,
            function,
            block,
            terminator,
            block_exit_env,
            live_locals_by_await,
            await_index,
            model,
        ),
        CoreTerminatorKind::Branch => record_select_paths(
            source_path,
            source,
            function,
            block,
            terminator,
            block_exit_env,
            replay,
            model,
        ),
        CoreTerminatorKind::Goto
        | CoreTerminatorKind::Return
        | CoreTerminatorKind::ErrorExit
        | CoreTerminatorKind::Panic
        | CoreTerminatorKind::OpaqueBoundary => {}
    }
}
