use std::collections::BTreeMap;

use crate::{CandidateAdmissionEvidence, ProofCertificate, ReplayGrade, VerificationError};

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
        verify_candidate_specific_gates(candidate)?;
    }
    Ok(())
}

fn verify_candidate_specific_gates(
    candidate: &CandidateAdmissionEvidence,
) -> Result<(), VerificationError> {
    let facts = candidate
        .evidence
        .iter()
        .map(|fact| (fact.key.as_str(), fact.value.as_str()))
        .collect::<BTreeMap<_, _>>();
    match candidate.id.as_str() {
        "S-32" => {
            require_fact(candidate, &facts, "target_rust", "target Rust")?;
            if facts.get("avoids_nightly").copied() != Some("true") {
                return Err(candidate_gate_error(
                    &candidate.id,
                    "nightly feature avoidance",
                ));
            }
        }
        "S-49" => {
            if facts.get("no_hidden_heap").copied() != Some("true") {
                return Err(candidate_gate_error(&candidate.id, "hidden heap"));
            }
            require_fact(candidate, &facts, "allocation_report", "allocation report")?;
            require_fact(candidate, &facts, "memory_budget", "memory budget")?;
        }
        "S-37+" => {
            require_fact(candidate, &facts, "cast_policy", "cast policy")?;
            require_fact(candidate, &facts, "debt_casts", "cast debt")?;
            if facts.get("strict_casts").copied() != Some("explicit") {
                return Err(candidate_gate_error(&candidate.id, "explicit cast"));
            }
        }
        _ => {}
    }
    Ok(())
}

fn require_fact(
    candidate: &CandidateAdmissionEvidence,
    facts: &BTreeMap<&str, &str>,
    key: &str,
    gate: &str,
) -> Result<(), VerificationError> {
    if facts.get(key).is_none() {
        return Err(candidate_gate_error(&candidate.id, gate));
    }
    Ok(())
}

fn candidate_gate_error(id: &str, gate: &str) -> VerificationError {
    VerificationError::CandidateAdmissionMissing {
        id: id.to_owned(),
        gate: gate.to_owned(),
    }
}
