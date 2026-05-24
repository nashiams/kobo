use std::collections::BTreeSet;

use crate::{
    stable_hash, BoundDeclaration, BoundDimension, BoundSource, BoundedCompleteness,
    BoundedProofEvidence, ProofCertificate, VerificationError,
};

pub fn normalized_bound_hash(evidence: &BoundedProofEvidence) -> String {
    stable_hash(&normalized_bound_material(evidence))
}

pub(crate) fn verify_bounded_evidence(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    for evidence in &certificate.bounded_evidence {
        verify_normalized_bound_hash(evidence)?;
        verify_proof_relevant_bounds(evidence)?;
        verify_bound_declarations(evidence)?;
        verify_complete_dimensions(evidence)?;
        verify_complete_loop_coverage(certificate, evidence)?;
        verify_bounded_wording(evidence)?;
        verify_complete_history_count(evidence)?;
        verify_canonical_histories(evidence)?;
    }
    Ok(())
}

fn verify_complete_loop_coverage(
    certificate: &ProofCertificate,
    evidence: &BoundedProofEvidence,
) -> Result<(), VerificationError> {
    if evidence.completeness != BoundedCompleteness::Complete {
        return Ok(());
    }
    let required_loop_ids = certificate
        .core
        .loop_facts
        .iter()
        .filter(|fact| fact.function == evidence.function)
        .map(|fact| fact.id.as_str())
        .chain(
            certificate
                .core
                .loop_exit_facts
                .iter()
                .filter(|fact| fact.function == evidence.function)
                .map(|fact| fact.id.as_str()),
        );
    for loop_id in required_loop_ids {
        if evidence.loop_ids.iter().any(|id| id == loop_id) {
            continue;
        }
        return Err(VerificationError::MissingLoopBackEdgeFact {
            loop_id: loop_id.to_owned(),
        });
    }
    Ok(())
}

fn verify_proof_relevant_bounds(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    if evidence.bounds.iter().any(|bound| bound.proof_relevant) {
        return Ok(());
    }
    Err(VerificationError::MissingBoundedProofDimension {
        evidence_id: evidence.id.clone(),
    })
}

fn verify_normalized_bound_hash(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    let expected = normalized_bound_hash(evidence);
    if evidence.normalized_bound_hash == expected {
        return Ok(());
    }
    Err(VerificationError::BoundedEvidenceHashMismatch {
        evidence_id: evidence.id.clone(),
        expected,
        observed: evidence.normalized_bound_hash.clone(),
    })
}

fn verify_bound_declarations(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    if evidence.completeness != BoundedCompleteness::Complete {
        return Ok(());
    }
    let expected = evidence
        .expected_complete_history_count
        .unwrap_or(evidence.enumerated_history_count);
    if has_complete_product_bounds(evidence, expected)
        && has_required_state_bounds(evidence)
        && expected == declared_history_product(evidence)
    {
        return Ok(());
    }
    Err(VerificationError::MissingBoundedProofDimension {
        evidence_id: evidence.id.clone(),
    })
}

fn verify_complete_dimensions(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    if evidence.completeness != BoundedCompleteness::Complete {
        return Ok(());
    }
    if !evidence.scheduler_dimensions.is_empty()
        && !evidence.fault_dimensions.is_empty()
        && !evidence.cancellation_points.is_empty()
        && evidence.pruned_histories.is_empty()
    {
        return Ok(());
    }
    Err(VerificationError::MissingBoundedProofDimension {
        evidence_id: evidence.id.clone(),
    })
}

fn verify_bounded_wording(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    let claims_bounded_proof = evidence.wording.contains("bounded proof");
    if evidence.completeness == BoundedCompleteness::Complete {
        let expected = format!(
            "bounded proof: all {} histories explored under declared bounds",
            evidence.enumerated_history_count
        );
        if evidence.wording == expected {
            return Ok(());
        }
        return Err(VerificationError::IncompleteBoundedEnumeration {
            evidence_id: evidence.id.clone(),
            completeness: evidence.completeness.as_str().to_owned(),
            wording: evidence.wording.clone(),
        });
    }
    if claims_bounded_proof {
        return Err(VerificationError::IncompleteBoundedEnumeration {
            evidence_id: evidence.id.clone(),
            completeness: evidence.completeness.as_str().to_owned(),
            wording: evidence.wording.clone(),
        });
    }
    if evidence.wording.contains("evidence only") {
        return Ok(());
    }
    Err(VerificationError::IncompleteBoundedEnumeration {
        evidence_id: evidence.id.clone(),
        completeness: evidence.completeness.as_str().to_owned(),
        wording: evidence.wording.clone(),
    })
}

fn verify_complete_history_count(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    if evidence.completeness != BoundedCompleteness::Complete {
        return Ok(());
    }
    let Some(expected) = evidence.expected_complete_history_count else {
        return Err(VerificationError::IncompleteBoundedEnumeration {
            evidence_id: evidence.id.clone(),
            completeness: evidence.completeness.as_str().to_owned(),
            wording: evidence.wording.clone(),
        });
    };
    if expected == evidence.enumerated_history_count {
        return Ok(());
    }
    Err(VerificationError::BoundedHistoryCountMismatch {
        evidence_id: evidence.id.clone(),
        expected,
        observed: evidence.enumerated_history_count,
    })
}

fn verify_canonical_histories(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    if evidence.completeness != BoundedCompleteness::Complete {
        return verify_history_hashes(evidence);
    }
    if evidence.canonical_histories.len() as u64 != evidence.enumerated_history_count {
        return Err(VerificationError::BoundedHistoryCountMismatch {
            evidence_id: evidence.id.clone(),
            expected: evidence.enumerated_history_count,
            observed: evidence.canonical_histories.len() as u64,
        });
    }
    verify_history_hashes(evidence)
}

fn has_bound(bounds: &[BoundDeclaration], dimension: BoundDimension, value: u64) -> bool {
    bounds
        .iter()
        .any(|bound| bound.proof_relevant && bound.dimension == dimension && bound.value == value)
}

fn has_complete_product_bounds(evidence: &BoundedProofEvidence, expected: u64) -> bool {
    has_bound(
        &evidence.bounds,
        BoundDimension::SchedulerHistories,
        expected,
    ) && bound_value(&evidence.bounds, BoundDimension::LoopIterations).is_some()
        && has_bound(
            &evidence.bounds,
            BoundDimension::FaultInjectionChoices,
            evidence.fault_dimensions.len() as u64,
        )
        && has_bound(
            &evidence.bounds,
            BoundDimension::CancellationPoints,
            evidence.cancellation_points.len() as u64,
        )
}

fn has_required_state_bounds(evidence: &BoundedProofEvidence) -> bool {
    required_state_dimensions()
        .into_iter()
        .all(|dimension| bound_value(&evidence.bounds, dimension).is_some())
}

fn required_state_dimensions() -> [BoundDimension; 5] {
    [
        BoundDimension::QueueCapacity,
        BoundDimension::MessageCount,
        BoundDimension::RetryAttempts,
        BoundDimension::TimeoutPaths,
        BoundDimension::ExternalBoundaryRecordings,
    ]
}

fn bound_value(bounds: &[BoundDeclaration], dimension: BoundDimension) -> Option<u64> {
    bounds
        .iter()
        .find(|bound| bound.proof_relevant && bound.dimension == dimension)
        .map(|bound| bound.value)
}

fn declared_history_product(evidence: &BoundedProofEvidence) -> u64 {
    let scheduler = evidence.scheduler_dimensions.len() as u64;
    let fault = evidence.fault_dimensions.len() as u64;
    let cancellation = evidence.cancellation_points.len() as u64;
    let loop_iterations = bound_value(&evidence.bounds, BoundDimension::LoopIterations)
        .map_or(0, numeric_bound_cardinality);
    let state = required_state_dimensions()
        .into_iter()
        .filter_map(|dimension| bound_value(&evidence.bounds, dimension))
        .map(numeric_bound_cardinality)
        .fold(1_u64, u64::saturating_mul);
    scheduler
        .saturating_mul(fault)
        .saturating_mul(cancellation)
        .saturating_mul(loop_iterations)
        .saturating_mul(state)
}

const fn numeric_bound_cardinality(value: u64) -> u64 {
    if value == 0 {
        1
    } else {
        value
    }
}

fn verify_history_hashes(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    let mut ids = BTreeSet::new();
    let mut dimension_keys = BTreeSet::new();
    for history in &evidence.canonical_histories {
        if !history_dimensions_match_bounds(evidence, history) {
            return Err(VerificationError::IncompleteBoundedEnumeration {
                evidence_id: evidence.id.clone(),
                completeness: evidence.completeness.as_str().to_owned(),
                wording: evidence.wording.clone(),
            });
        }
        let material = history_material(history);
        let dimension_key = history_dimension_key(history);
        let expected = stable_hash(&material);
        if history.history_hash != expected
            || !ids.insert(history.id.as_str())
            || !dimension_keys.insert(dimension_key)
        {
            return Err(VerificationError::IncompleteBoundedEnumeration {
                evidence_id: evidence.id.clone(),
                completeness: evidence.completeness.as_str().to_owned(),
                wording: evidence.wording.clone(),
            });
        }
    }
    Ok(())
}

fn history_dimensions_match_bounds(
    evidence: &BoundedProofEvidence,
    history: &crate::BoundedHistoryEvidence,
) -> bool {
    evidence
        .scheduler_dimensions
        .iter()
        .any(|dimension| dimension == &history.scheduler)
        && evidence
            .fault_dimensions
            .iter()
            .any(|dimension| dimension == &history.fault)
        && evidence
            .cancellation_points
            .iter()
            .any(|dimension| dimension == &history.cancellation)
        && numeric_history_value_matches_bound(
            history.loop_iteration,
            bound_value(&evidence.bounds, BoundDimension::LoopIterations),
        )
        && numeric_history_value_matches_bound(
            history.queue_capacity,
            bound_value(&evidence.bounds, BoundDimension::QueueCapacity),
        )
        && numeric_history_value_matches_bound(
            history.message_count,
            bound_value(&evidence.bounds, BoundDimension::MessageCount),
        )
        && numeric_history_value_matches_bound(
            history.retry_attempts,
            bound_value(&evidence.bounds, BoundDimension::RetryAttempts),
        )
        && numeric_history_value_matches_bound(
            history.timeout_path,
            bound_value(&evidence.bounds, BoundDimension::TimeoutPaths),
        )
        && numeric_history_value_matches_bound(
            history.external_boundary_recording,
            bound_value(&evidence.bounds, BoundDimension::ExternalBoundaryRecordings),
        )
}

const fn numeric_history_value_matches_bound(value: u64, bound: Option<u64>) -> bool {
    match bound {
        Some(0) => value == 0,
        Some(bound) => value >= 1 && value <= bound,
        None => false,
    }
}

fn history_material(history: &crate::BoundedHistoryEvidence) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        history.id,
        history.scheduler,
        history.fault,
        history.cancellation,
        history.loop_iteration,
        history.queue_capacity,
        history.message_count,
        history.retry_attempts,
        history.timeout_path,
        history.external_boundary_recording,
    )
}

fn history_dimension_key(history: &crate::BoundedHistoryEvidence) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}",
        history.scheduler,
        history.fault,
        history.cancellation,
        history.loop_iteration,
        history.queue_capacity,
        history.message_count,
        history.retry_attempts,
        history.timeout_path,
        history.external_boundary_recording,
    )
}

fn normalized_bound_material(evidence: &BoundedProofEvidence) -> String {
    format!(
        "function={};enumerated={};expected={:?};bounds={};scheduler={};fault={};cancellation={}",
        evidence.function,
        evidence.enumerated_history_count,
        evidence.expected_complete_history_count,
        bounds_material(&evidence.bounds),
        list_material(&evidence.scheduler_dimensions),
        list_material(&evidence.fault_dimensions),
        list_material(&evidence.cancellation_points),
    )
}

fn bounds_material(bounds: &[BoundDeclaration]) -> String {
    bounds
        .iter()
        .map(|bound| {
            format!(
                "{}:{}:{}:{}",
                bound_dimension_name(&bound.dimension),
                bound.value,
                bound_source_name(&bound.source),
                bound.proof_relevant
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn list_material(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("{}:{value}", value.len()))
        .collect::<Vec<_>>()
        .join(",")
}

fn bound_dimension_name(dimension: &BoundDimension) -> &'static str {
    match dimension {
        BoundDimension::LoopIterations => "loop_iterations",
        BoundDimension::QueueCapacity => "queue_capacity",
        BoundDimension::MessageCount => "message_count",
        BoundDimension::SchedulerHistories => "scheduler_histories",
        BoundDimension::CancellationPoints => "cancellation_points",
        BoundDimension::RetryAttempts => "retry_attempts",
        BoundDimension::TimeoutPaths => "timeout_paths",
        BoundDimension::FaultInjectionChoices => "fault_injection_choices",
        BoundDimension::ExternalBoundaryRecordings => "external_boundary_recordings",
    }
}

fn bound_source_name(source: &BoundSource) -> &'static str {
    match source {
        BoundSource::Ward => "ward",
        BoundSource::Template => "template",
        BoundSource::Adapter => "adapter",
        BoundSource::Summary => "summary",
    }
}
