use std::collections::BTreeSet;

use crate::{
    stable_hash, BoundedCompleteness, BoundedProofEvidence, ProofCertificate, VerificationError,
};

pub(crate) fn verify_bounded_evidence(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    for evidence in &certificate.bounded_evidence {
        verify_proof_relevant_bounds(evidence)?;
        verify_complete_dimensions(evidence)?;
        verify_bounded_wording(evidence)?;
        verify_complete_history_count(evidence)?;
        verify_canonical_histories(evidence)?;
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

fn verify_history_hashes(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    let mut ids = BTreeSet::new();
    for history in &evidence.canonical_histories {
        let material = format!(
            "{}:{}:{}:{}",
            history.id, history.scheduler, history.fault, history.cancellation
        );
        let expected = stable_hash(&material);
        if history.history_hash != expected || !ids.insert(history.id.as_str()) {
            return Err(VerificationError::IncompleteBoundedEnumeration {
                evidence_id: evidence.id.clone(),
                completeness: evidence.completeness.as_str().to_owned(),
                wording: evidence.wording.clone(),
            });
        }
    }
    Ok(())
}
