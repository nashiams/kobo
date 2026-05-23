use crate::{ProofCertificate, ReplayGrade, VerificationError};

pub(crate) fn verify_candidate_admission(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    for candidate in &certificate.candidate_admission {
        if candidate.status != "graduate" {
            continue;
        }
        if candidate
            .inspect_visibility
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            return Err(candidate_gate_error(&candidate.id, "inspect visibility"));
        }
        if candidate
            .manual_rust_equivalent
            .as_deref()
            .unwrap_or("")
            .is_empty()
        {
            return Err(candidate_gate_error(
                &candidate.id,
                "manual Rust equivalent",
            ));
        }
        if !candidate.strict_compatible {
            return Err(candidate_gate_error(&candidate.id, "Strict-compatible"));
        }
        if candidate.whole_ecosystem_modeling_required {
            return Err(candidate_gate_error(
                &candidate.id,
                "whole-ecosystem modeling",
            ));
        }
        if candidate.diagnostic_snapshots.is_empty() {
            return Err(candidate_gate_error(&candidate.id, "diagnostic snapshot"));
        }
        if candidate.replay_related
            && (candidate.replay_grade.is_none()
                || !candidate.adapter_confidence.iter().all(|adapter| {
                    adapter.replay_grade
                        == candidate
                            .replay_grade
                            .clone()
                            .unwrap_or(ReplayGrade::NotReplayable)
                }))
        {
            return Err(candidate_gate_error(
                &candidate.id,
                "replay grade and adapter confidence",
            ));
        }
    }
    Ok(())
}

fn candidate_gate_error(id: &str, gate: &str) -> VerificationError {
    VerificationError::CandidateAdmissionMissing {
        id: id.to_owned(),
        gate: gate.to_owned(),
    }
}
