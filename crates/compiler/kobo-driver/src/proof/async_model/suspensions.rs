use std::collections::BTreeMap;

use kobo_ir::{CoreBlock, CoreFunction, CoreTerminator};
use kobo_proof::{
    AsyncModelEvidence, CancelEdgeEvidence, FutureStateLocalEvidence,
    FutureStateObligationEvidence, ObligationStatus, SourceSpan, SuspensionStateEvidence,
    TimeoutCancelEdgeEvidence,
};

use super::super::obligations::env_states;
use super::super::source_spans::{source_span_for_binding, source_span_from_kobo};

struct AwaitEdges {
    resume_edge: String,
    cancel_edge: String,
}

pub(super) fn record_await_suspension(
    source_path: &str,
    source: &str,
    function: &CoreFunction,
    block: &CoreBlock,
    terminator: &CoreTerminator,
    block_exit_env: &BTreeMap<String, ObligationStatus>,
    live_locals_by_await: &[Vec<String>],
    await_index: &mut usize,
    model: &mut AsyncModelEvidence,
) {
    let suspension_id = format!("{}:{}:{}", function.name, block.id, terminator.id);
    let edges = await_edges(terminator);
    let source_span = source_span_from_kobo(source_path, source, terminator.source_span);
    model.suspension_states.push(SuspensionStateEvidence {
        id: suspension_id.clone(),
        function: function.name.clone(),
        block: block.id.clone(),
        terminator_kind: "await".to_owned(),
        boundary: terminator.boundary.clone(),
        resume_edge: edges.resume_edge,
        cancel_edge: edges.cancel_edge.clone(),
        source_span: source_span.clone(),
    });
    record_future_drop_cancel_edge(
        function,
        block,
        &suspension_id,
        &edges.cancel_edge,
        &source_span,
        model,
    );
    record_future_state_locals(
        source_path,
        source,
        &suspension_id,
        live_locals_by_await,
        await_index,
        model,
    );
    record_future_state_obligations(&suspension_id, block_exit_env, &source_span, model);
    record_timeout_cancel_edge(
        function,
        terminator,
        suspension_id,
        edges.cancel_edge,
        source_span,
        model,
    );
}

fn await_edges(terminator: &CoreTerminator) -> AwaitEdges {
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
    AwaitEdges {
        resume_edge,
        cancel_edge,
    }
}

fn record_future_drop_cancel_edge(
    function: &CoreFunction,
    block: &CoreBlock,
    suspension_id: &str,
    cancel_edge: &str,
    source_span: &SourceSpan,
    model: &mut AsyncModelEvidence,
) {
    model.cancel_edges.push(CancelEdgeEvidence {
        id: format!("{suspension_id}:future-drop"),
        function: function.name.clone(),
        from: block.id.clone(),
        to: cancel_edge.to_owned(),
        reason: "future_drop".to_owned(),
        source_span: source_span.clone(),
    });
}

fn record_future_state_locals(
    source_path: &str,
    source: &str,
    suspension_id: &str,
    live_locals_by_await: &[Vec<String>],
    await_index: &mut usize,
    model: &mut AsyncModelEvidence,
) {
    let live_locals = live_locals_by_await
        .get(*await_index)
        .cloned()
        .unwrap_or_default();
    *await_index += 1;
    for local in &live_locals {
        model.future_state_locals.push(FutureStateLocalEvidence {
            binding: local.clone(),
            suspension_state: suspension_id.to_owned(),
            source_span: source_span_for_binding(source_path, source, local),
        });
    }
}

fn record_future_state_obligations(
    suspension_id: &str,
    block_exit_env: &BTreeMap<String, ObligationStatus>,
    source_span: &SourceSpan,
    model: &mut AsyncModelEvidence,
) {
    for state in env_states(block_exit_env)
        .into_iter()
        .filter(|state| state.state == ObligationStatus::Owned)
    {
        model
            .future_state_obligations
            .push(FutureStateObligationEvidence {
                binding: state.binding,
                state: state.state,
                suspension_state: suspension_id.to_owned(),
                source_span: source_span.clone(),
            });
    }
}

fn record_timeout_cancel_edge(
    function: &CoreFunction,
    terminator: &CoreTerminator,
    suspension_id: String,
    cancel_edge: String,
    source_span: SourceSpan,
    model: &mut AsyncModelEvidence,
) {
    if terminator.boundary.as_deref() != Some("tokio::time::timeout") {
        return;
    }
    model.timeout_cancel_edges.push(TimeoutCancelEdgeEvidence {
        id: format!("{suspension_id}:timeout"),
        function: function.name.clone(),
        suspension_state: suspension_id,
        source: "tokio::time::timeout".to_owned(),
        cancel_edge,
        source_span,
    });
}
