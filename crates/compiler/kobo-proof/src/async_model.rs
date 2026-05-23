use std::collections::{BTreeMap, BTreeSet};

use crate::{
    stable_hash, CancelEdgeEvidence, ObligationStatus, ProofCertificate, SelectPathEvidence,
    SuspensionStateEvidence, VerificationError,
};

use crate::obligation::{block_obligation_envs, event_state_map};

pub(crate) fn verify_cancel_edges(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let mut await_blocks = BTreeSet::<String>::new();
    let mut cancel_blocks = BTreeSet::<String>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await" {
            await_blocks.insert(edge.from.clone());
            if edge.to == "await_cancel" {
                cancel_blocks.insert(edge.from.clone());
            }
        }
    }
    for block in await_blocks {
        if !cancel_blocks.contains(&block) {
            return Err(VerificationError::MissingCancelEdge { block });
        }
    }
    Ok(())
}

pub(crate) fn verify_async_model(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let await_edges = await_edges(certificate);
    let suspensions = suspensions_by_id(certificate);
    let suspension_blocks = suspensions
        .values()
        .map(|state| state.block.as_str())
        .collect::<BTreeSet<_>>();

    for (function, block, edge_targets) in &await_edges {
        let Some(suspension) = suspensions
            .values()
            .find(|state| state.function == *function && state.block == *block)
        else {
            return Err(VerificationError::MissingAsyncCancelEvidence {
                block: block.clone(),
            });
        };
        if suspension.terminator_kind != "await"
            || !edge_targets.contains(suspension.resume_edge.as_str())
            || !edge_targets.contains(suspension.cancel_edge.as_str())
        {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "suspension_states".to_owned(),
                id: suspension.id.clone(),
            });
        }
    }

    let cancel_evidence = certificate
        .core
        .async_model
        .cancel_edges
        .iter()
        .map(cancel_edge_key)
        .collect::<BTreeSet<_>>();
    for (function, block, edge_targets) in &await_edges {
        if edge_targets.contains("await_cancel")
            && !cancel_evidence.contains(&(function.as_str(), block.as_str(), "await_cancel"))
        {
            return Err(VerificationError::MissingAsyncCancelEvidence {
                block: block.clone(),
            });
        }
    }

    for local in &certificate.core.async_model.future_state_locals {
        if !suspensions.contains_key(local.suspension_state.as_str()) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_locals".to_owned(),
                id: local.binding.clone(),
            });
        }
    }

    verify_future_state_obligations(certificate, &suspensions)?;
    verify_select_paths(certificate)?;
    verify_timeout_cancel_edges(certificate, &suspensions)?;
    verify_spawned_task_obligations(certificate)?;

    for suspension in suspensions.values() {
        if !suspension_blocks.contains(suspension.block.as_str()) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "suspension_states".to_owned(),
                id: suspension.id.clone(),
            });
        }
    }

    Ok(())
}

fn verify_future_state_obligations(
    certificate: &ProofCertificate,
    suspensions: &BTreeMap<&str, &SuspensionStateEvidence>,
) -> Result<(), VerificationError> {
    let block_envs = block_obligation_envs(certificate);
    let mut actual = BTreeMap::<(&str, &str), ObligationStatus>::new();
    for obligation in &certificate.core.async_model.future_state_obligations {
        if !suspensions.contains_key(obligation.suspension_state.as_str()) {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "future_state_obligations".to_owned(),
                id: obligation.binding.clone(),
            });
        }
        actual.insert(
            (
                obligation.suspension_state.as_str(),
                obligation.binding.as_str(),
            ),
            obligation.state.clone(),
        );
    }

    for suspension in suspensions.values() {
        let env = block_envs
            .get(&suspension.block)
            .cloned()
            .unwrap_or_default();
        for (binding, expected_state) in env {
            if expected_state != ObligationStatus::Owned {
                continue;
            }
            let observed = actual.get(&(suspension.id.as_str(), binding.as_str()));
            if observed != Some(&ObligationStatus::Owned) {
                return Err(VerificationError::MissingFutureStateObligation {
                    binding,
                    suspension_state: suspension.id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn verify_select_paths(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let mut branch_blocks = BTreeSet::<(&str, &str)>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "branch" {
            branch_blocks.insert((edge.function.as_str(), edge.from.as_str()));
        }
    }
    let paths = certificate
        .core
        .async_model
        .select_paths
        .iter()
        .collect::<Vec<_>>();
    for (function, block) in branch_blocks {
        for path_kind in ["winner", "loser_cancel"] {
            let Some(path) = paths.iter().copied().find(|path| {
                path.function == function
                    && path.branch_block == block
                    && path.path_kind == path_kind
            }) else {
                return Err(VerificationError::MissingSelectPathEvidence {
                    block: block.to_owned(),
                    path_kind: path_kind.to_owned(),
                });
            };
            verify_obligation_result_hash(certificate, path)?;
        }
    }
    Ok(())
}

fn verify_timeout_cancel_edges(
    certificate: &ProofCertificate,
    suspensions: &BTreeMap<&str, &SuspensionStateEvidence>,
) -> Result<(), VerificationError> {
    let cancel_evidence = certificate
        .core
        .async_model
        .cancel_edges
        .iter()
        .map(cancel_edge_key)
        .collect::<BTreeSet<_>>();
    for timeout in &certificate.core.async_model.timeout_cancel_edges {
        let Some(suspension) = suspensions.get(timeout.suspension_state.as_str()) else {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "timeout_cancel_edges".to_owned(),
                id: timeout.id.clone(),
            });
        };
        if timeout.source != "tokio::time::timeout"
            || timeout.cancel_edge != suspension.cancel_edge
            || !cancel_evidence.contains(&(
                suspension.function.as_str(),
                suspension.block.as_str(),
                timeout.cancel_edge.as_str(),
            ))
        {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "timeout_cancel_edges".to_owned(),
                id: timeout.id.clone(),
            });
        }
    }
    Ok(())
}

fn verify_spawned_task_obligations(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let exit_env = event_state_map(&certificate.exit_env);
    for task in &certificate.core.async_model.spawned_task_obligations {
        let has_resolution_policy = task.required_resolution.iter().any(|resolution| {
            matches!(
                resolution.as_str(),
                "await" | "join" | "abort" | "detach" | "detach-with-policy" | "transfer"
            )
        });
        if !has_resolution_policy {
            return Err(VerificationError::AsyncEvidenceMismatch {
                field: "spawned_task_obligations".to_owned(),
                id: task.binding.clone(),
            });
        }
        if exit_env
            .get(&task.binding)
            .is_some_and(ObligationStatus::is_unresolved_exit)
        {
            return Err(VerificationError::UnresolvedExitObligation {
                binding: task.binding.clone(),
            });
        }
    }
    Ok(())
}

fn verify_obligation_result_hash(
    certificate: &ProofCertificate,
    path: &SelectPathEvidence,
) -> Result<(), VerificationError> {
    let observed = canonical_obligation_result_hash(&certificate.exit_env);
    if observed != path.obligation_result_hash {
        return Err(VerificationError::AsyncEvidenceMismatch {
            field: "select_paths.obligation_result_hash".to_owned(),
            id: path.id.clone(),
        });
    }
    Ok(())
}

fn canonical_obligation_result_hash(states: &[crate::ObligationState]) -> String {
    let mut statuses = states
        .iter()
        .map(|state| state.state.as_str())
        .collect::<Vec<_>>();
    statuses.sort_unstable();
    stable_hash(&format!("obligation-result:{statuses:?}"))
}

fn await_edges<'a>(certificate: &'a ProofCertificate) -> Vec<(String, String, BTreeSet<&'a str>)> {
    let mut grouped = BTreeMap::<(String, String), BTreeSet<&'a str>>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await" {
            grouped
                .entry((edge.function.clone(), edge.from.clone()))
                .or_default()
                .insert(edge.to.as_str());
        }
    }
    grouped
        .into_iter()
        .map(|((function, block), targets)| (function, block, targets))
        .collect()
}

fn suspensions_by_id(certificate: &ProofCertificate) -> BTreeMap<&str, &SuspensionStateEvidence> {
    certificate
        .core
        .async_model
        .suspension_states
        .iter()
        .map(|state| (state.id.as_str(), state))
        .collect()
}

fn cancel_edge_key(edge: &CancelEdgeEvidence) -> (&str, &str, &str) {
    (edge.function.as_str(), edge.from.as_str(), edge.to.as_str())
}
