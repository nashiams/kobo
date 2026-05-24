use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::{
    BoundedCompleteness, InvariantPreservation, InvariantTier, ObligationStatus, ProofCertificate,
    UserInvariantPredicate, VerificationError,
};

pub(crate) fn verify_loop_invariants(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let block_entry_envs = crate::obligation::block_obligation_entry_envs(certificate)?;
    let block_exit_envs = crate::obligation::block_obligation_envs(certificate)?;
    for invariant in &certificate.loop_invariants {
        verify_invariant_template(certificate, invariant)?;
        verify_user_invariant_fact(certificate, invariant)?;
        let checked_bindings = invariant_checked_bindings(invariant);
        verify_preservation_status(invariant)?;
        verify_invariant_created_bindings(certificate, invariant)?;
        verify_entry_states_match_core_replay(invariant, &block_entry_envs, &checked_bindings)?;
        reject_entry_leaks(invariant, &checked_bindings)?;
        reject_back_edge_leaks(invariant, &checked_bindings)?;
        verify_back_edge_states_match_core_replay(invariant, &block_exit_envs, &checked_bindings)?;
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
                && evidence.loop_ids.iter().any(|loop_id| loop_id == &fact.id)
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
    let loop_id = invariant_loop_id(certificate, invariant);
    let has_core_fact = certificate.core.loop_facts.iter().any(|fact| {
        fact.id == invariant.id
            && fact.function == invariant.function
            && fact.loop_id == loop_id
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
    let loop_id = invariant_loop_id(certificate, invariant);
    let expected = certificate
        .obligation_events
        .iter()
        .filter(|event| event.kind == crate::ObligationEventKind::Create)
        .filter(|event| event.loop_regions.iter().any(|region| region == loop_id))
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
        reason: "invariant obligation scope does not match Core loop-region membership".to_owned(),
    })
}

fn invariant_loop_id<'a>(
    certificate: &'a ProofCertificate,
    invariant: &'a crate::LoopInvariantEvidence,
) -> &'a str {
    if !invariant.loop_id.is_empty() {
        return invariant.loop_id.as_str();
    }
    certificate
        .core
        .loop_facts
        .iter()
        .find(|fact| fact.id == invariant.id && fact.function == invariant.function)
        .map(|fact| fact.loop_id.as_str())
        .unwrap_or_default()
}

fn verify_entry_states_match_core_replay(
    invariant: &crate::LoopInvariantEvidence,
    block_entry_envs: &std::collections::BTreeMap<String, crate::obligation::ObligationEnv>,
    checked_bindings: &BTreeSet<String>,
) -> Result<(), VerificationError> {
    let Some(actual_env) = block_entry_envs.get(&invariant.entry_block) else {
        return Ok(());
    };
    let expected = actual_env
        .iter()
        .filter(|(binding, _)| checked_bindings.contains(binding.as_str()))
        .map(|(binding, state)| (binding.clone(), state.clone()))
        .collect::<BTreeMap<_, _>>();
    let observed = invariant
        .entry_states
        .iter()
        .filter(|state| checked_bindings.contains(state.binding.as_str()))
        .map(|state| (state.binding.clone(), state.state.clone()))
        .collect::<BTreeMap<_, _>>();
    if expected == observed {
        return Ok(());
    }
    Err(VerificationError::InvariantNotPreserved {
        loop_id: invariant.id.clone(),
        reason: "invariant entry states do not match Core replay".to_owned(),
    })
}

fn verify_back_edge_states_match_core_replay(
    invariant: &crate::LoopInvariantEvidence,
    block_exit_envs: &std::collections::BTreeMap<String, crate::obligation::ObligationEnv>,
    checked_bindings: &BTreeSet<String>,
) -> Result<(), VerificationError> {
    let Some(actual_env) = block_exit_envs.get(&invariant.back_edge_source) else {
        return Ok(());
    };
    let expected = actual_env
        .iter()
        .filter(|(binding, _)| checked_bindings.contains(binding.as_str()))
        .map(|(binding, state)| (binding.clone(), state.clone()))
        .collect::<BTreeMap<_, _>>();
    let observed = invariant
        .back_edge_states
        .iter()
        .filter(|state| checked_bindings.contains(state.binding.as_str()))
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

fn reject_entry_leaks(
    invariant: &crate::LoopInvariantEvidence,
    checked_bindings: &BTreeSet<String>,
) -> Result<(), VerificationError> {
    for state in &invariant.entry_states {
        if checked_bindings.contains(state.binding.as_str())
            && is_unresolved_back_edge_state(&state.state)
        {
            return Err(VerificationError::InvariantNotPreserved {
                loop_id: invariant.id.clone(),
                reason: format!(
                    "invariant entry is not true for unresolved binding {}",
                    state.binding
                ),
            });
        }
    }
    Ok(())
}

fn verify_invariant_template(
    certificate: &ProofCertificate,
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    if invariant.tier != InvariantTier::Inferred {
        verify_binding_template_hashes(certificate, invariant)?;
        return Ok(());
    }
    verify_binding_template_coverage(invariant)?;
    verify_binding_template_hashes(certificate, invariant)?;
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

fn verify_binding_template_coverage(
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    let expected = invariant
        .obligations_created
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let observed = invariant
        .binding_templates
        .iter()
        .map(|template| template.binding.clone())
        .collect::<BTreeSet<_>>();
    if expected == observed && observed.len() == invariant.binding_templates.len() {
        return Ok(());
    }
    Err(VerificationError::MissingInvariantTemplate {
        loop_id: invariant.id.clone(),
    })
}

fn verify_binding_template_hashes(
    certificate: &ProofCertificate,
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    let template_hashes = certificate
        .template_hashes
        .iter()
        .map(|hash| (hash.id.as_str(), hash.hash.as_str()))
        .collect::<BTreeMap<_, _>>();
    for template in &invariant.binding_templates {
        if template.id.is_empty()
            || template.version.is_empty()
            || template.schema_hash.is_empty()
            || template.binding.is_empty()
            || template.obligation_kind.is_empty()
            || template.lifecycle_owner.is_empty()
        {
            return Err(VerificationError::MissingInvariantTemplate {
                loop_id: invariant.id.clone(),
            });
        }
        let Some(expected_hash) = template_hashes.get(template.id.as_str()) else {
            return Err(VerificationError::MissingInvariantTemplate {
                loop_id: invariant.id.clone(),
            });
        };
        if template.schema_hash != *expected_hash {
            return Err(VerificationError::TemplateHashMismatch {
                id: template.id.clone(),
                expected: (*expected_hash).to_owned(),
                observed: template.schema_hash.clone(),
            });
        }
    }
    Ok(())
}

fn verify_user_invariant_fact(
    certificate: &ProofCertificate,
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    if invariant.tier != InvariantTier::User {
        return reject_non_user_invariant_fact(invariant);
    }
    let Some(fact) = invariant.user_fact.as_ref() else {
        return Err(user_invariant_error(
            invariant,
            "user invariant is missing typed fact evidence",
        ));
    };
    let loop_id = invariant_loop_id(certificate, invariant);
    if fact.loop_id != loop_id {
        return Err(user_invariant_error(
            invariant,
            "user invariant fact loop id does not match Core loop fact",
        ));
    }
    if fact.obligation_kind.trim().is_empty() {
        return Err(user_invariant_error(
            invariant,
            "user invariant fact is missing obligation kind",
        ));
    }
    let expected_expression = match fact.predicate {
        UserInvariantPredicate::NoPending => format!("no_pending({})", fact.obligation_kind),
    };
    if invariant.expression != expected_expression {
        return Err(user_invariant_error(
            invariant,
            "user invariant expression does not match typed fact",
        ));
    }
    verify_user_invariant_domain(invariant, &fact.domain_bindings)?;
    verify_user_invariant_template_refs(invariant)?;
    Ok(())
}

fn reject_non_user_invariant_fact(
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    if invariant.user_fact.is_none() {
        return Ok(());
    }
    Err(user_invariant_error(
        invariant,
        "typed user invariant facts are only valid for user invariants",
    ))
}

fn verify_user_invariant_domain(
    invariant: &crate::LoopInvariantEvidence,
    domain_bindings: &[String],
) -> Result<(), VerificationError> {
    let domain = domain_bindings.iter().collect::<BTreeSet<_>>();
    if domain.len() != domain_bindings.len() || domain.is_empty() {
        return Err(user_invariant_error(
            invariant,
            "user invariant fact has an invalid binding domain",
        ));
    }
    let created = invariant
        .obligations_created
        .iter()
        .collect::<BTreeSet<_>>();
    if !created.is_subset(&domain) {
        return Err(user_invariant_error(
            invariant,
            "user invariant fact does not cover created obligations",
        ));
    }
    Ok(())
}

fn verify_user_invariant_template_refs(
    invariant: &crate::LoopInvariantEvidence,
) -> Result<(), VerificationError> {
    let Some(fact) = invariant.user_fact.as_ref() else {
        return Ok(());
    };
    let Some(template) = invariant.template.as_ref() else {
        if fact.template_id.is_none()
            && fact.template_version.is_none()
            && fact.lifecycle_owner.is_none()
        {
            return Ok(());
        }
        return Err(user_invariant_error(
            invariant,
            "user invariant fact references a missing lifecycle template",
        ));
    };
    if fact.template_id.as_deref() != Some(template.id.as_str())
        || fact.template_version.as_deref() != Some(template.version.as_str())
        || fact.lifecycle_owner.as_deref() != Some(template.lifecycle_owner.as_str())
        || fact.obligation_kind != template.obligation_kind
    {
        return Err(user_invariant_error(
            invariant,
            "user invariant fact does not match lifecycle template evidence",
        ));
    }
    Ok(())
}

fn invariant_checked_bindings(invariant: &crate::LoopInvariantEvidence) -> BTreeSet<String> {
    if let Some(fact) = invariant.user_fact.as_ref() {
        return fact.domain_bindings.iter().cloned().collect();
    }
    invariant.obligations_created.iter().cloned().collect()
}

fn user_invariant_error(
    invariant: &crate::LoopInvariantEvidence,
    reason: impl Into<String>,
) -> VerificationError {
    VerificationError::InvariantNotPreserved {
        loop_id: invariant.id.clone(),
        reason: reason.into(),
    }
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
    checked_bindings: &BTreeSet<String>,
) -> Result<(), VerificationError> {
    for state in &invariant.back_edge_states {
        if checked_bindings.contains(state.binding.as_str())
            && is_unresolved_back_edge_state(&state.state)
        {
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
