use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::{
    BoundedCompleteness, InvariantPreservation, InvariantTier, ObligationStatus, ProofCertificate,
    VerificationError,
};

pub(crate) fn verify_loop_invariants(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let block_exit_envs = crate::obligation::block_obligation_envs(certificate)?;
    for invariant in &certificate.loop_invariants {
        verify_invariant_template(invariant)?;
        verify_preservation_status(invariant)?;
        verify_invariant_created_bindings(certificate, invariant)?;
        reject_back_edge_leaks(invariant)?;
        verify_back_edge_states_match_core_replay(invariant, &block_exit_envs)?;
        verify_loop_back_edge_shape(certificate, invariant)?;
    }
    verify_every_loop_fact_has_proof(certificate)?;
    Ok(())
}

fn verify_every_loop_fact_has_proof(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    for fact in &certificate.core.loop_facts {
        let has_invariant = certificate.loop_invariants.iter().any(|invariant| {
            invariant.id == fact.id
                && invariant.function == fact.function
                && invariant.preservation == InvariantPreservation::Preserved
        });
        let has_complete_bounded_evidence = certificate.bounded_evidence.iter().any(|evidence| {
            evidence.function == fact.function
                && evidence.completeness == BoundedCompleteness::Complete
        });
        if has_invariant || has_complete_bounded_evidence {
            continue;
        }
        return Err(VerificationError::MissingLoopBackEdgeFact {
            loop_id: fact.id.clone(),
        });
    }
    Ok(())
}

fn verify_loop_back_edge_shape(
    certificate: &ProofCertificate,
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    let has_core_fact = certificate.core.loop_facts.iter().any(|fact| {
        fact.id == invariant.id
            && fact.function == invariant.function
            && fact.entry_block == invariant.entry_block
            && fact.back_edge_source == invariant.back_edge_source
            && fact.back_edge_target == invariant.back_edge_target
    });
    if has_core_fact {
        return Ok(());
    }
    Err(VerificationError::MissingLoopBackEdgeFact {
        loop_id: invariant.id.clone(),
    })
}

fn verify_invariant_created_bindings(
    certificate: &ProofCertificate,
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    let Some(entry_index) = block_index(&invariant.entry_block) else {
        return Ok(());
    };
    let Some(back_edge_index) = block_index(&invariant.back_edge_source) else {
        return Ok(());
    };
    let expected = certificate
        .obligation_events
        .iter()
        .filter(|event| event.kind == crate::ObligationEventKind::Create)
        .filter(|event| {
            statement_index(&event.id)
                .is_some_and(|index| entry_index <= index && index <= back_edge_index)
        })
        .filter_map(|event| event.binding.clone())
        .collect::<BTreeSet<_>>();
    let observed = invariant
        .obligations_created
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if expected == observed {
        return Ok(());
    }
    Err(VerificationError::InvariantNotPreserved {
        loop_id: invariant.id.clone(),
        reason: "invariant obligation scope does not match Core loop range".to_owned(),
    })
}

fn verify_back_edge_states_match_core_replay(
    invariant: &crate::LoopInvariantEvidence,
    block_exit_envs: &std::collections::BTreeMap<String, crate::obligation::ObligationEnv>,
) -> Result<(), VerificationError> {
    let Some(actual_env) = block_exit_envs.get(&invariant.back_edge_source) else {
        return Ok(());
    };
    let created = invariant
        .obligations_created
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = actual_env
        .iter()
        .filter(|(binding, _)| created.contains(binding.as_str()))
        .map(|(binding, state)| (binding.clone(), state.clone()))
        .collect::<BTreeMap<_, _>>();
    let observed = invariant
        .back_edge_states
        .iter()
        .filter(|state| created.contains(state.binding.as_str()))
        .map(|state| (state.binding.clone(), state.state.clone()))
        .collect::<BTreeMap<_, _>>();
    if expected == observed {
        return Ok(());
    }
    Err(VerificationError::InvariantNotPreserved {
        loop_id: invariant.id.clone(),
        reason: "invariant back-edge states do not match Core replay".to_owned(),
    })
}

fn verify_invariant_template(
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    if invariant.tier != InvariantTier::Inferred {
        return Ok(());
    }
    let Some(template) = invariant.template.as_ref() else {
        return Err(VerificationError::MissingInvariantTemplate {
            loop_id: invariant.id.clone(),
        });
    };
    if template.id.is_empty() || template.version.is_empty() || template.schema_hash.is_empty() {
        return Err(VerificationError::MissingInvariantTemplate {
            loop_id: invariant.id.clone(),
        });
    }
    Ok(())
}

fn verify_preservation_status(
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    if invariant.preservation == InvariantPreservation::Preserved {
        return Ok(());
    }
    Err(VerificationError::InvariantNotPreserved {
        loop_id: invariant.id.clone(),
        reason: invariant
            .downgrade_reason
            .clone()
            .unwrap_or_else(|| "invariant preservation failed".to_owned()),
    })
}

fn reject_back_edge_leaks(
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    let created = invariant
        .obligations_created
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for state in &invariant.back_edge_states {
        if created.contains(state.binding.as_str()) && is_unresolved_back_edge_state(&state.state) {
            return Err(VerificationError::LoopBackEdgeLeak {
                loop_id: invariant.id.clone(),
                binding: state.binding.clone(),
            });
        }
    }
    Ok(())
}

fn is_unresolved_back_edge_state(status: &ObligationStatus) -> bool {
    matches!(
        status,
        ObligationStatus::Owned | ObligationStatus::Moved | ObligationStatus::BranchUnresolved
    )
}

fn statement_index(id: &str) -> Option<usize> {
    id.strip_prefix("stmt-")?.parse().ok()
}

fn block_index(id: &str) -> Option<usize> {
    id.strip_prefix("bb")?.parse().ok()
}
