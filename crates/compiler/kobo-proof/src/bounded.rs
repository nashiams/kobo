use crate::{BoundedCompleteness, BoundedProofEvidence, ProofCertificate, VerificationError};

pub(crate) fn verify_bounded_evidence(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    for evidence in &certificate.bounded_evidence {
        verify_proof_relevant_bounds(evidence)?;
        verify_bounded_wording(evidence)?;
        verify_complete_history_count(evidence)?;
    }
    Ok(())
}

fn verify_proof_relevant_bounds(
    evidence: &BoundedProofEvidence,
) -> Result<(), VerificationError> {
    if evidence.bounds.iter().any(|bound| bound.proof_relevant) {
        return Ok(());
    }
    Err(VerificationError::MissingBoundedProofDimension {
        evidence_id: evidence.id.clone(),
    })
}

fn verify_bounded_wording(evidence: &BoundedProofEvidence) -> Result<(), VerificationError> {
    let claims_bounded_proof = evidence.wording.contains("bounded proof");
    if evidence.completeness == BoundedCompleteness::Complete {
        return Ok(());
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
