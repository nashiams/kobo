use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ObligationEvent, ObligationEventKind, ObligationState, ObligationStatus, ProofCertificate,
    VerificationError,
};

pub(crate) type ObligationEnv = BTreeMap<String, ObligationStatus>;

pub(crate) fn replay_obligation_events(
    certificate: &ProofCertificate,
) -> Result<usize, VerificationError> {
    let mut env = ObligationEnv::new();
    for event in &certificate.obligation_events {
        verify_event_state_before(event, &env)?;
        apply_obligation_event(event, &mut env);
        verify_event_state_after(event, &env)?;
    }
    verify_exit_env(certificate, &env)?;
    reject_unresolved_exit(&env)?;
    Ok(certificate.obligation_events.len())
}

pub(crate) fn verify_cfg_edge_transitions(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let node_ids = certificate
        .core
        .cfg_nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let block_envs = block_obligation_envs(certificate);
    for edge in &certificate.core.cfg_edges {
        if !node_ids.contains(edge.from.as_str()) {
            return Err(VerificationError::CfgEdgeTransitionMismatch {
                edge: edge.id.clone(),
                reason: format!("unknown source block {}", edge.from),
            });
        }
        if edge.to.starts_with("bb") && !node_ids.contains(edge.to.as_str()) {
            return Err(VerificationError::CfgEdgeTransitionMismatch {
                edge: edge.id.clone(),
                reason: format!("unknown target block {}", edge.to),
            });
        }
        if modeled_exit_target(edge.to.as_str()) {
            if let Some(env) = block_envs.get(&edge.from) {
                reject_unresolved_exit_on_edge(&edge.id, env)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn block_obligation_envs(
    certificate: &ProofCertificate,
) -> BTreeMap<String, ObligationEnv> {
    let events_by_index = certificate
        .obligation_events
        .iter()
        .filter_map(|event| statement_index(&event.id).map(|index| (index, event)))
        .collect::<BTreeMap<_, _>>();
    let mut nodes = certificate.core.cfg_nodes.iter().collect::<Vec<_>>();
    nodes.sort_by_key(|node| block_index(&node.id).unwrap_or(usize::MAX));

    let mut env = ObligationEnv::new();
    let mut envs = BTreeMap::new();
    for node in nodes {
        if let Some(index) = block_index(&node.id) {
            if let Some(event) = events_by_index.get(&index) {
                apply_obligation_event(event, &mut env);
            }
        }
        envs.insert(node.id.clone(), env.clone());
    }
    envs
}

pub(crate) fn event_state_map(states: &[ObligationState]) -> ObligationEnv {
    states
        .iter()
        .map(|state| (state.binding.clone(), state.state.clone()))
        .collect()
}

pub(crate) fn apply_obligation_event(event: &ObligationEvent, env: &mut ObligationEnv) {
    let Some(binding) = event.binding.as_ref() else {
        return;
    };
    let state = match event.kind {
        ObligationEventKind::Create => ObligationStatus::Owned,
        ObligationEventKind::Discharge => ObligationStatus::Resolved,
        ObligationEventKind::Transfer => ObligationStatus::Transferred,
        ObligationEventKind::Move => ObligationStatus::Moved,
        ObligationEventKind::BranchUnresolved => ObligationStatus::BranchUnresolved,
        ObligationEventKind::Escape => ObligationStatus::Escaped,
        ObligationEventKind::UnsupportedContainer | ObligationEventKind::Call => return,
    };
    env.insert(binding.clone(), state);
}

fn verify_event_state_before(
    event: &ObligationEvent,
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    verify_expected_states(&event.state_before, env)
}

fn verify_event_state_after(
    event: &ObligationEvent,
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    verify_expected_states(&event.state_after, env)
}

fn verify_exit_env(
    certificate: &ProofCertificate,
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    verify_expected_states(&certificate.exit_env, env)
}

fn verify_expected_states(
    expected_states: &[ObligationState],
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    for (binding, expected_state) in event_state_map(expected_states) {
        let Some(observed_state) = env.get(&binding) else {
            return Err(VerificationError::ObligationReplayMismatch {
                binding,
                expected: expected_state.as_str().to_owned(),
                observed: String::new(),
            });
        };
        if observed_state != &expected_state {
            if expected_state == ObligationStatus::Resolved
                && observed_state == &ObligationStatus::Owned
            {
                return Err(VerificationError::RemovedDischarge { binding });
            }
            return Err(VerificationError::ObligationReplayMismatch {
                binding,
                expected: expected_state.as_str().to_owned(),
                observed: observed_state.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

fn reject_unresolved_exit(env: &ObligationEnv) -> Result<(), VerificationError> {
    for (binding, state) in env {
        if state.is_unresolved_exit() {
            return Err(VerificationError::UnresolvedExitObligation {
                binding: binding.clone(),
            });
        }
    }
    Ok(())
}

fn reject_unresolved_exit_on_edge(
    edge_id: &str,
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    for (binding, state) in env {
        if state.is_unresolved_exit() {
            return Err(VerificationError::CfgEdgeTransitionMismatch {
                edge: edge_id.to_owned(),
                reason: format!("unresolved obligation {binding} reaches modeled edge exit"),
            });
        }
    }
    Ok(())
}

fn modeled_exit_target(target: &str) -> bool {
    matches!(target, "return" | "error_exit" | "panic")
}

fn statement_index(id: &str) -> Option<usize> {
    id.strip_prefix("stmt-")?.parse().ok()
}

fn block_index(id: &str) -> Option<usize> {
    id.strip_prefix("bb")?.parse().ok()
}
