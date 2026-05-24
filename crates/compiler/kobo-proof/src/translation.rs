use std::collections::{BTreeMap, BTreeSet};

use crate::{
    trace, trace_material_hash, GeneratedTraceEvent, HashEvidence, ProofCertificate,
    SourceMapAnchorStatus, TranslationValidationStatus, VerificationError,
};

pub(crate) fn verify_translation_validation(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    if certificate.core_obligation_trace.is_empty() && certificate.generated_rust_trace.is_empty() {
        return verify_empty_trace_status(certificate);
    }
    if certificate.generated_rust_trace.is_empty()
        && matches!(
            certificate.translation_validation.status,
            TranslationValidationStatus::CoreOnly | TranslationValidationStatus::NotGenerated
        )
    {
        return Ok(());
    }
    let generated_by_core_id = generated_trace_by_core_id(&certificate.generated_rust_trace)?;
    verify_core_events_have_generated_matches(certificate, &generated_by_core_id)?;
    verify_generated_events_have_core_matches(certificate)?;
    verify_trace_hashes(certificate)?;
    verify_trace_status(certificate, TranslationValidationStatus::Validated)
}

fn verify_empty_trace_status(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    if matches!(
        certificate.translation_validation.status,
        TranslationValidationStatus::CoreOnly | TranslationValidationStatus::NotGenerated
    ) {
        return Ok(());
    }
    Err(VerificationError::TranslationValidationStatusMismatch {
        expected: TranslationValidationStatus::CoreOnly.as_str().to_owned(),
        observed: certificate
            .translation_validation
            .status
            .as_str()
            .to_owned(),
    })
}

fn generated_trace_by_core_id(
    generated_trace: &[GeneratedTraceEvent],
) -> Result<BTreeMap<&str, &GeneratedTraceEvent>, VerificationError> {
    let mut generated_by_core_id = BTreeMap::new();
    for generated_event in trace::sorted_generated_trace(generated_trace) {
        if generated_by_core_id
            .insert(generated_event.core_event_id.as_str(), generated_event)
            .is_some()
        {
            return Err(VerificationError::TranslationTraceExtraEvent {
                generated_event_id: generated_event.id.clone(),
            });
        }
    }
    Ok(generated_by_core_id)
}

fn verify_core_events_have_generated_matches(
    certificate: &ProofCertificate,
    generated_by_core_id: &BTreeMap<&str, &GeneratedTraceEvent>,
) -> Result<(), VerificationError> {
    for core_event in trace::sorted_core_trace(&certificate.core_obligation_trace) {
        let Some(generated_event) = generated_by_core_id.get(core_event.id.as_str()) else {
            return Err(VerificationError::TranslationTraceMissingEvent {
                core_event_id: core_event.id.clone(),
            });
        };
        verify_event_identity(core_event, generated_event)?;
        verify_source_map_anchor(generated_event)?;
    }
    Ok(())
}

fn verify_generated_events_have_core_matches(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let core_ids = certificate
        .core_obligation_trace
        .iter()
        .map(|event| event.id.as_str())
        .collect::<BTreeSet<_>>();
    for generated_event in &certificate.generated_rust_trace {
        if core_ids.contains(generated_event.core_event_id.as_str()) {
            continue;
        }
        return Err(VerificationError::TranslationTraceExtraEvent {
            generated_event_id: generated_event.id.clone(),
        });
    }
    Ok(())
}

fn verify_event_identity(
    core_event: &crate::CoreTraceEvent,
    generated_event: &GeneratedTraceEvent,
) -> Result<(), VerificationError> {
    if core_event.order != generated_event.order {
        return Err(VerificationError::TranslationTraceOrderMismatch {
            core_event_id: core_event.id.clone(),
            expected: core_event.order,
            observed: generated_event.order,
        });
    }
    if core_event.kind != generated_event.kind {
        return Err(VerificationError::TranslationTraceKindMismatch {
            core_event_id: core_event.id.clone(),
            expected: core_event.kind.as_str().to_owned(),
            observed: generated_event.kind.as_str().to_owned(),
        });
    }
    if core_event.binding != generated_event.binding {
        return Err(VerificationError::TranslationTraceBindingMismatch {
            core_event_id: core_event.id.clone(),
            expected: trace::optional_text(&core_event.binding),
            observed: trace::optional_text(&generated_event.binding),
        });
    }
    if core_event.template_id != generated_event.template_id
        || core_event.template_version != generated_event.template_version
    {
        return Err(VerificationError::TranslationTraceTemplateMismatch {
            core_event_id: core_event.id.clone(),
            expected: format!(
                "{}@{}",
                trace::optional_text(&core_event.template_id),
                trace::optional_text(&core_event.template_version)
            ),
            observed: format!(
                "{}@{}",
                trace::optional_text(&generated_event.template_id),
                trace::optional_text(&generated_event.template_version)
            ),
        });
    }
    Ok(())
}

fn verify_source_map_anchor(
    generated_event: &GeneratedTraceEvent,
) -> Result<(), VerificationError> {
    if generated_event.source_map_anchor.status == SourceMapAnchorStatus::Mapped
        && !generated_event.source_map_anchor.id.is_empty()
    {
        return Ok(());
    }
    Err(VerificationError::TranslationSourceMapAnchorMismatch {
        generated_event_id: generated_event.id.clone(),
        anchor_id: generated_event.source_map_anchor.id.clone(),
        status: generated_event.source_map_anchor.status.as_str().to_owned(),
    })
}

fn verify_trace_status(
    certificate: &ProofCertificate,
    expected_status: TranslationValidationStatus,
) -> Result<(), VerificationError> {
    if certificate.translation_validation.status == expected_status
        && certificate.translation_validation.mismatches.is_empty()
    {
        return Ok(());
    }
    Err(VerificationError::TranslationValidationStatusMismatch {
        expected: expected_status.as_str().to_owned(),
        observed: certificate
            .translation_validation
            .status
            .as_str()
            .to_owned(),
    })
}

fn verify_trace_hashes(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    verify_trace_hash(
        "core_obligation_trace",
        &certificate.trace_hashes,
        trace_material_hash(&certificate.core_obligation_trace),
    )?;
    verify_trace_hash(
        "generated_rust_trace",
        &certificate.trace_hashes,
        trace_material_hash(&certificate.generated_rust_trace),
    )
}

fn verify_trace_hash(
    trace_id: &str,
    hashes: &[HashEvidence],
    expected: Result<String, serde_json::Error>,
) -> Result<(), VerificationError> {
    let expected = expected.map_err(|error| VerificationError::Parse {
        message: format!("failed to hash {trace_id}: {error}"),
    })?;
    let observed = hashes
        .iter()
        .find(|hash| hash.id == trace_id)
        .map(|hash| hash.hash.clone())
        .unwrap_or_default();
    if observed == expected {
        return Ok(());
    }
    Err(VerificationError::TranslationTraceHashMismatch {
        trace_id: trace_id.to_owned(),
        expected,
        observed,
    })
}
