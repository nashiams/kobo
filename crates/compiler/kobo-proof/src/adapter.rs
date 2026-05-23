use std::collections::BTreeSet;

use crate::{AdapterConfidence, BoundaryPolicy, ProofCertificate, ReplayGrade, VerificationError};

pub(crate) fn verify_adapter_confidence(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    verify_exact_replay_has_adapter_evidence(certificate)?;
    for adapter in &certificate.adapter_confidence {
        if certificate.replay_grade == ReplayGrade::Exact
            && adapter.confidence != AdapterConfidence::Exact
        {
            return Err(VerificationError::ExactReplayWithAdapterConfidence {
                adapter: adapter.adapter.clone(),
                confidence: adapter_confidence_name(&adapter.confidence).to_owned(),
            });
        }
        if adapter.confidence == AdapterConfidence::MetadataOnly
            && matches!(
                certificate.replay_grade,
                ReplayGrade::Exact | ReplayGrade::Partial
            )
        {
            return Err(VerificationError::MetadataOnlyAdapterReplayable {
                adapter: adapter.adapter.clone(),
            });
        }
        if adapter.confidence == AdapterConfidence::Sampled && adapter.outcome != "probing_pass" {
            return Err(VerificationError::SampledAdapterWithoutProbingPass {
                adapter: adapter.adapter.clone(),
            });
        }
        if adapter_version_is_stale(adapter.version.as_deref())
            && !matches!(
                certificate.replay_grade,
                ReplayGrade::Debt | ReplayGrade::NotReplayable
            )
        {
            return Err(VerificationError::StaleAdapterReplayable {
                adapter: adapter.adapter.clone(),
            });
        }
    }
    Ok(())
}

fn verify_exact_replay_has_adapter_evidence(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    if certificate.replay_grade != ReplayGrade::Exact {
        return Ok(());
    }
    let adapter_boundaries = certificate
        .adapter_confidence
        .iter()
        .map(|adapter| adapter.boundary.as_str())
        .collect::<BTreeSet<_>>();
    for assumption in &certificate.boundary_assumptions {
        if requires_adapter_evidence(&assumption.policy)
            && !adapter_boundaries.contains(assumption.boundary.as_str())
        {
            return Err(VerificationError::ExactReplayMissingAdapterEvidence {
                boundary: assumption.boundary.clone(),
            });
        }
    }
    Ok(())
}

fn requires_adapter_evidence(policy: &BoundaryPolicy) -> bool {
    matches!(
        policy,
        BoundaryPolicy::Record
            | BoundaryPolicy::Activity
            | BoundaryPolicy::Model
            | BoundaryPolicy::Stub
    )
}

fn adapter_confidence_name(confidence: &AdapterConfidence) -> &'static str {
    match confidence {
        AdapterConfidence::Exact => "exact",
        AdapterConfidence::Modeled => "modeled",
        AdapterConfidence::Sampled => "sampled",
        AdapterConfidence::MetadataOnly => "metadata-only",
    }
}

fn adapter_version_is_stale(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return true;
    };
    version == "0.0.0" || version.contains("stale")
}
