use std::collections::BTreeMap;

use kobo_ir::{CoreBlock, CoreFunction, CoreTerminator};
use kobo_proof::{
    stable_hash, AsyncModelEvidence, ObligationState, ObligationStatus, SelectPathEvidence,
    SourceSpan,
};

use super::super::obligations::{env_states, FunctionObligationReplay};
use super::super::source_spans::source_span_from_kobo;

pub(super) fn record_select_paths(
    source_path: &str,
    source: &str,
    function: &CoreFunction,
    block: &CoreBlock,
    terminator: &CoreTerminator,
    block_exit_env: &BTreeMap<String, ObligationStatus>,
    replay: &FunctionObligationReplay,
    model: &mut AsyncModelEvidence,
) {
    let source_span = source_span_from_kobo(source_path, source, terminator.source_span);
    let cancelled_obligations = env_states(block_exit_env)
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
        record_select_path_variants(
            function,
            block,
            terminator,
            branch_target,
            &obligation_results,
            &cancelled_obligations,
            &source_span,
            model,
        );
    }
}

fn record_select_path_variants(
    function: &CoreFunction,
    block: &CoreBlock,
    terminator: &CoreTerminator,
    branch_target: &str,
    obligation_results: &[ObligationState],
    cancelled_obligations: &[ObligationState],
    source_span: &SourceSpan,
    model: &mut AsyncModelEvidence,
) {
    for path_kind in ["winner", "loser_cancel"] {
        let cancelled_obligations = (path_kind == "loser_cancel")
            .then(|| cancelled_obligations.to_vec())
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
            obligation_results: obligation_results.to_vec(),
            cancelled_obligations: cancelled_obligations.clone(),
            obligation_result_hash: canonical_select_result_hash(
                branch_target,
                path_kind,
                obligation_results,
                &cancelled_obligations,
            ),
            source_span: source_span.clone(),
        });
    }
}

fn canonical_select_result_hash(
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
