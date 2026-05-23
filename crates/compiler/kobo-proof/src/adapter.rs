use crate::{AdapterConfidence, ProofCertificate, ReplayGrade, VerificationError};

pub(crate) fn verify_adapter_confidence(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
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
