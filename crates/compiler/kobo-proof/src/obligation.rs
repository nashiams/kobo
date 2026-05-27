use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    CoreCfgEdge, InvariantPreservation, ObligationEvent, ObligationEventKind, ObligationState,
    ObligationStatus, ProofCertificate, VerificationError,
};

pub(crate) type ObligationEnv = BTreeMap<String, ObligationStatus>;

struct CfgReplay {
    block_entry_envs: BTreeMap<String, ObligationEnv>,
    block_exit_envs: BTreeMap<String, ObligationEnv>,
    terminal_envs: Vec<ObligationEnv>,
    checked_events: usize,
}

pub(crate) fn replay_obligation_events(
    certificate: &ProofCertificate,
) -> Result<usize, VerificationError> {
    let replay = replay_cfg_obligations(certificate)?;
    for env in &replay.terminal_envs {
        verify_exit_env(certificate, env)?;
        reject_unresolved_exit(env)?;
    }
    Ok(replay.checked_events)
}

pub(crate) fn verify_cfg_edge_transitions(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    replay_cfg_obligations(certificate).map(|_| ())
}

pub(crate) fn block_obligation_envs(
    certificate: &ProofCertificate,
) -> Result<BTreeMap<String, ObligationEnv>, VerificationError> {
    replay_cfg_obligations(certificate).map(|replay| replay.block_exit_envs)
}

pub(crate) fn block_obligation_entry_envs(
    certificate: &ProofCertificate,
) -> Result<BTreeMap<String, ObligationEnv>, VerificationError> {
    replay_cfg_obligations(certificate).map(|replay| replay.block_entry_envs)
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

fn replay_cfg_obligations(certificate: &ProofCertificate) -> Result<CfgReplay, VerificationError> {
    let nodes = certificate
        .core
        .cfg_nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let Some(entry_node) = certificate
        .core
        .cfg_nodes
        .iter()
        .min_by_key(|node| block_index(&node.id).unwrap_or(usize::MAX))
    else {
        return Ok(CfgReplay {
            block_entry_envs: BTreeMap::new(),
            block_exit_envs: BTreeMap::new(),
            terminal_envs: Vec::new(),
            checked_events: 0,
        });
    };
    let events_by_block = certificate
        .obligation_events
        .iter()
        .filter_map(|event| statement_index(&event.id).map(|index| (format!("bb{index}"), event)))
        .collect::<BTreeMap<_, _>>();
    let edges_by_source = edges_by_source(&certificate.core.cfg_edges);

    let mut block_entry_envs = BTreeMap::<String, ObligationEnv>::new();
    let mut block_exit_envs = BTreeMap::<String, ObligationEnv>::new();
    let mut terminal_envs = Vec::new();
    let mut checked_event_ids = BTreeSet::<String>::new();
    let mut queued = VecDeque::new();

    block_entry_envs.insert(
        entry_node.id.clone(),
        event_state_map(&certificate.entry_env),
    );
    queued.push_back(entry_node.id.clone());

    while let Some(block_id) = queued.pop_front() {
        let Some(_node) = nodes.get(block_id.as_str()) else {
            continue;
        };
        let entry_env = block_entry_envs.get(&block_id).cloned().unwrap_or_default();
        let exit_env = replay_block_event(
            &block_id,
            &entry_env,
            &events_by_block,
            &mut checked_event_ids,
        )?;
        block_exit_envs.insert(block_id.clone(), exit_env.clone());

        let outgoing = edges_by_source
            .get(block_id.as_str())
            .cloned()
            .unwrap_or_default();
        let mut has_block_successor = false;
        for edge in outgoing {
            let target = core_successor_target(edge.to.as_str());
            if modeled_exit_target(target.as_str()) {
                reject_unresolved_exit_on_edge(&edge.id, &exit_env)?;
                terminal_envs.push(exit_env.clone());
                continue;
            }
            if !target.starts_with("bb") {
                continue;
            }
            has_block_successor = true;
            if !nodes.contains_key(target.as_str()) {
                return Err(VerificationError::CfgEdgeTransitionMismatch {
                    edge: edge.id.clone(),
                    reason: format!("unknown target block {target}"),
                });
            }
            match block_entry_envs.get(target.as_str()) {
                Some(existing) if existing == &exit_env => {}
                Some(_) if is_invariant_back_edge(certificate, edge) => {}
                Some(existing) => {
                    return Err(VerificationError::CfgEdgeTransitionMismatch {
                        edge: edge.id.clone(),
                        reason: format!(
                            "target block {} expects {}, observed {}",
                            target,
                            format_env(existing),
                            format_env(&exit_env)
                        ),
                    });
                }
                None => {
                    block_entry_envs.insert(target.clone(), exit_env.clone());
                    queued.push_back(target);
                }
            }
        }
        if !has_block_successor {
            terminal_envs.push(exit_env);
        }
    }

    for edge in &certificate.core.cfg_edges {
        if !nodes.contains_key(edge.from.as_str()) {
            return Err(VerificationError::CfgEdgeTransitionMismatch {
                edge: edge.id.clone(),
                reason: format!("unknown source block {}", edge.from),
            });
        }
    }
    for event in &certificate.obligation_events {
        if !checked_event_ids.contains(&event.id) {
            return Err(VerificationError::CfgEdgeTransitionMismatch {
                edge: event.id.clone(),
                reason: "obligation event is unreachable from CFG entry".to_owned(),
            });
        }
    }

    Ok(CfgReplay {
        block_entry_envs,
        block_exit_envs,
        terminal_envs,
        checked_events: checked_event_ids.len(),
    })
}

fn is_invariant_back_edge(certificate: &ProofCertificate, edge: &CoreCfgEdge) -> bool {
    certificate.loop_invariants.iter().any(|invariant| {
        invariant.function == edge.function
            && invariant.back_edge_source == edge.from
            && invariant.back_edge_target == edge.to
            && invariant.preservation == InvariantPreservation::Preserved
    })
}

fn replay_block_event(
    block_id: &str,
    entry_env: &ObligationEnv,
    events_by_block: &BTreeMap<String, &ObligationEvent>,
    checked_event_ids: &mut BTreeSet<String>,
) -> Result<ObligationEnv, VerificationError> {
    let Some(event) = events_by_block.get(block_id) else {
        return Ok(entry_env.clone());
    };
    verify_env_exact(&event.state_before, entry_env)?;
    verify_event_precondition(event, entry_env)?;
    let mut observed = entry_env.clone();
    apply_obligation_event(event, &mut observed);
    verify_env_exact(&event.state_after, &observed)?;
    checked_event_ids.insert(event.id.clone());
    Ok(observed)
}

fn verify_event_precondition(
    event: &ObligationEvent,
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    let Some(binding) = event.binding.as_ref() else {
        return Ok(());
    };
    let observed = env.get(binding);
    let allowed = match event.kind {
        ObligationEventKind::Create => observed.is_none(),
        ObligationEventKind::Transfer
        | ObligationEventKind::Move
        | ObligationEventKind::BranchUnresolved => {
            matches!(observed, Some(ObligationStatus::Owned))
        }
        ObligationEventKind::Discharge => {
            matches!(
                observed,
                Some(ObligationStatus::Owned | ObligationStatus::Transferred)
            )
        }
        ObligationEventKind::Escape => {
            matches!(
                observed,
                Some(
                    ObligationStatus::Owned
                        | ObligationStatus::Transferred
                        | ObligationStatus::Resolved
                )
            )
        }
        ObligationEventKind::UnsupportedContainer | ObligationEventKind::Call => true,
    };
    if allowed {
        return Ok(());
    }
    Err(VerificationError::ObligationReplayMismatch {
        binding: binding.clone(),
        expected: expected_precondition(&event.kind).to_owned(),
        observed: observed
            .map(ObligationStatus::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

fn expected_precondition(kind: &ObligationEventKind) -> &'static str {
    match kind {
        ObligationEventKind::Create => "absent",
        ObligationEventKind::Transfer
        | ObligationEventKind::Move
        | ObligationEventKind::BranchUnresolved => "owned",
        ObligationEventKind::Discharge => "owned|transferred",
        ObligationEventKind::Escape => "owned|transferred|resolved",
        ObligationEventKind::UnsupportedContainer | ObligationEventKind::Call => "any",
    }
}

fn edges_by_source(edges: &[CoreCfgEdge]) -> BTreeMap<&str, Vec<&CoreCfgEdge>> {
    let mut grouped = BTreeMap::<&str, Vec<&CoreCfgEdge>>::new();
    for edge in edges {
        grouped.entry(edge.from.as_str()).or_default().push(edge);
    }
    grouped
}

fn verify_env_exact(
    expected_states: &[ObligationState],
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    let expected = event_state_map(expected_states);
    if expected == *env {
        return Ok(());
    }
    for (binding, expected_state) in &expected {
        let Some(observed_state) = env.get(binding) else {
            return Err(VerificationError::ObligationReplayMismatch {
                binding: binding.clone(),
                expected: expected_state.as_str().to_owned(),
                observed: String::new(),
            });
        };
        if observed_state != expected_state {
            if expected_state == &ObligationStatus::Resolved
                && observed_state == &ObligationStatus::Owned
            {
                return Err(VerificationError::RemovedDischarge {
                    binding: binding.clone(),
                });
            }
            return Err(VerificationError::ObligationReplayMismatch {
                binding: binding.clone(),
                expected: expected_state.as_str().to_owned(),
                observed: observed_state.as_str().to_owned(),
            });
        }
    }
    let Some((binding, observed_state)) = env
        .iter()
        .find(|(binding, _)| !expected.contains_key(*binding))
    else {
        return Ok(());
    };
    Err(VerificationError::ObligationReplayMismatch {
        binding: binding.clone(),
        expected: String::new(),
        observed: observed_state.as_str().to_owned(),
    })
}

fn verify_exit_env(
    certificate: &ProofCertificate,
    env: &ObligationEnv,
) -> Result<(), VerificationError> {
    verify_env_exact(&certificate.exit_env, env)
}

fn reject_unresolved_exit(env: &ObligationEnv) -> Result<(), VerificationError> {
    for (binding, state) in env {
        if state.is_unresolved_exit() {
            return Err(VerificationError::UnresolvedExitObligation {
                binding: binding.clone(),
                state: state.as_str().to_owned(),
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
                reason: format!(
                    "unresolved obligation {binding} reaches modeled edge exit as {}",
                    state.as_str()
                ),
            });
        }
    }
    Ok(())
}

fn modeled_exit_target(target: &str) -> bool {
    matches!(target, "return" | "error_exit" | "panic" | "break_exit")
}

fn core_successor_target(target: &str) -> String {
    let Some(payload) = target.strip_prefix("loop_exit:") else {
        return target.to_owned();
    };
    let parts = payload.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [_, _, _, exit_target] => (*exit_target).to_owned(),
        [_, _, exit_target] => (*exit_target).to_owned(),
        [kind, ..] => format!("{kind}_exit"),
        [] => "break_exit".to_owned(),
    }
}

fn format_env(env: &ObligationEnv) -> String {
    env.iter()
        .map(|(binding, state)| format!("{binding}:{}", state.as_str()))
        .collect::<Vec<_>>()
        .join(",")
}

fn statement_index(id: &str) -> Option<usize> {
    id.strip_prefix("stmt-")?.parse().ok()
}

fn block_index(id: &str) -> Option<usize> {
    id.strip_prefix("bb")?.parse().ok()
}
