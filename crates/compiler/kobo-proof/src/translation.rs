use std::collections::{BTreeMap, BTreeSet};

use crate::{
    trace, trace_material_hash, GeneratedTraceEvent, HashEvidence, ProofCertificate,
    SourceMapAnchorStatus, TranslationValidationStatus, VerificationError,
};

pub(crate) fn verify_translation_validation(
    certificate: &ProofCertificate,
    source_map_json: Option<&str>,
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
    let source_map = parse_required_source_map(source_map_json, &certificate.generated_rust_trace)?;
    verify_core_events_have_generated_matches(certificate, &generated_by_core_id, &source_map)?;
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
    source_map: &SourceMapValidation,
) -> Result<(), VerificationError> {
    let mut used_lowering_records = BTreeSet::new();
    for core_event in trace::sorted_core_trace(&certificate.core_obligation_trace) {
        let Some(generated_event) = generated_by_core_id.get(core_event.id.as_str()) else {
            return Err(VerificationError::TranslationTraceMissingEvent {
                core_event_id: core_event.id.clone(),
            });
        };
        verify_event_identity(core_event, generated_event)?;
        verify_source_map_anchor(generated_event, &source_map.anchors)?;
        verify_lowering_trace_record(generated_event, source_map, &mut used_lowering_records)?;
    }
    verify_no_extra_lowering_trace_records(source_map, &used_lowering_records)?;
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
    source_map: &BTreeMap<String, SourceMapAnchorRecord>,
) -> Result<(), VerificationError> {
    if generated_event.source_map_anchor.status == SourceMapAnchorStatus::Mapped
        && !generated_event.source_map_anchor.id.is_empty()
    {
        return verify_source_map_anchor_record(generated_event, source_map);
    }
    Err(VerificationError::TranslationSourceMapAnchorMismatch {
        generated_event_id: generated_event.id.clone(),
        anchor_id: generated_event.source_map_anchor.id.clone(),
        status: generated_event.source_map_anchor.status.as_str().to_owned(),
    })
}

#[derive(Clone, Debug)]
struct SourceMapAnchorRecord {
    core_event_id: Option<String>,
    kobo_start: usize,
    kobo_end: usize,
    rs_line: usize,
    rs_start: usize,
    rs_end: usize,
}

#[derive(Clone, Debug)]
struct LoweringTraceRecord {
    id: String,
    core_event_id: String,
    function: String,
    kind: String,
    binding: Option<String>,
    order: u64,
    source_map_entry_id: String,
    rs_line: usize,
    rs_start: usize,
    rs_end: usize,
    kobo_start: usize,
    kobo_end: usize,
    lowering_phase: String,
    template_id: Option<String>,
    template_version: Option<String>,
}

#[derive(Clone, Debug)]
struct SourceMapValidation {
    anchors: BTreeMap<String, SourceMapAnchorRecord>,
    lowering_trace: Vec<LoweringTraceRecord>,
}

fn parse_required_source_map(
    source_map_json: Option<&str>,
    generated_trace: &[GeneratedTraceEvent],
) -> Result<SourceMapValidation, VerificationError> {
    let Some(source_map_json) = source_map_json else {
        return Err(VerificationError::TranslationSourceMapAnchorMismatch {
            generated_event_id: generated_trace
                .first()
                .map(|event| event.id.clone())
                .unwrap_or_default(),
            anchor_id: generated_trace
                .first()
                .map(|event| event.source_map_anchor.id.clone())
                .unwrap_or_default(),
            status: "missing source map".to_owned(),
        });
    };
    parse_source_map(source_map_json).ok_or_else(|| {
        VerificationError::TranslationSourceMapAnchorMismatch {
            generated_event_id: generated_trace
                .first()
                .map(|event| event.id.clone())
                .unwrap_or_default(),
            anchor_id: generated_trace
                .first()
                .map(|event| event.source_map_anchor.id.clone())
                .unwrap_or_default(),
            status: "invalid source map".to_owned(),
        }
    })
}

fn parse_source_map(source_map_json: &str) -> Option<SourceMapValidation> {
    let value: serde_json::Value = serde_json::from_str(source_map_json).ok()?;
    let mappings = value["x_kobo_mappings"].as_array()?;
    let mut anchors = BTreeMap::new();
    for mapping in mappings {
        let id = mapping["id"].as_str()?.to_owned();
        anchors.insert(
            id,
            SourceMapAnchorRecord {
                core_event_id: mapping["core_event_id"].as_str().map(str::to_owned),
                kobo_start: mapping["kobo_span"]["start"].as_u64()? as usize,
                kobo_end: mapping["kobo_span"]["end"].as_u64()? as usize,
                rs_line: mapping["rs_span"]["line"].as_u64()? as usize,
                rs_start: mapping["rs_span"]["column_start"].as_u64()? as usize,
                rs_end: mapping["rs_span"]["column_end"].as_u64()? as usize,
            },
        );
    }
    let lowering_trace = value["lowering_trace"]
        .as_array()?
        .iter()
        .map(parse_lowering_trace_record)
        .collect::<Option<Vec<_>>>()?;
    Some(SourceMapValidation {
        anchors,
        lowering_trace,
    })
}

fn parse_lowering_trace_record(value: &serde_json::Value) -> Option<LoweringTraceRecord> {
    Some(LoweringTraceRecord {
        id: value["id"].as_str()?.to_owned(),
        core_event_id: value["core_event_id"].as_str()?.to_owned(),
        function: value["function"].as_str()?.to_owned(),
        kind: value["kind"].as_str()?.to_owned(),
        binding: value["binding"].as_str().map(str::to_owned),
        order: value["order"].as_u64()?,
        source_map_entry_id: value["source_map_entry_id"].as_str()?.to_owned(),
        rs_line: value["rs_span"]["line"].as_u64()? as usize,
        rs_start: value["rs_span"]["column_start"].as_u64()? as usize,
        rs_end: value["rs_span"]["column_end"].as_u64()? as usize,
        kobo_start: value["kobo_span"]["start"].as_u64()? as usize,
        kobo_end: value["kobo_span"]["end"].as_u64()? as usize,
        lowering_phase: value["lowering_phase"].as_str()?.to_owned(),
        template_id: value["template_id"].as_str().map(str::to_owned),
        template_version: value["template_version"].as_str().map(str::to_owned),
    })
}

fn verify_source_map_anchor_record(
    generated_event: &GeneratedTraceEvent,
    source_map: &BTreeMap<String, SourceMapAnchorRecord>,
) -> Result<(), VerificationError> {
    let Some(record) = source_map.get(&generated_event.source_map_anchor.id) else {
        return Err(VerificationError::TranslationSourceMapAnchorMismatch {
            generated_event_id: generated_event.id.clone(),
            anchor_id: generated_event.source_map_anchor.id.clone(),
            status: "missing".to_owned(),
        });
    };
    let anchor = &generated_event.source_map_anchor;
    if source_map_anchor_core_event_matches(record, generated_event)
        && record.kobo_start == anchor.kobo_span.start
        && record.kobo_end == anchor.kobo_span.end
        && record.rs_line == anchor.generated_span.line
        && record.rs_start == anchor.generated_span.start
        && record.rs_end == anchor.generated_span.end
    {
        return Ok(());
    }
    Err(VerificationError::TranslationSourceMapAnchorMismatch {
        generated_event_id: generated_event.id.clone(),
        anchor_id: generated_event.source_map_anchor.id.clone(),
        status: "stale".to_owned(),
    })
}

fn source_map_anchor_core_event_matches(
    record: &SourceMapAnchorRecord,
    generated_event: &GeneratedTraceEvent,
) -> bool {
    record.core_event_id.as_deref() == Some(generated_event.core_event_id.as_str())
}

fn verify_lowering_trace_record(
    generated_event: &GeneratedTraceEvent,
    source_map: &SourceMapValidation,
    used_lowering_records: &mut BTreeSet<usize>,
) -> Result<(), VerificationError> {
    if let Some((index, _)) =
        source_map
            .lowering_trace
            .iter()
            .enumerate()
            .find(|(index, record)| {
                !used_lowering_records.contains(index)
                    && lowering_trace_record_matches(generated_event, record)
            })
    {
        used_lowering_records.insert(index);
        return Ok(());
    }
    Err(VerificationError::TranslationSourceMapAnchorMismatch {
        generated_event_id: generated_event.id.clone(),
        anchor_id: generated_event.source_map_anchor.id.clone(),
        status: "missing lowering trace event".to_owned(),
    })
}

fn verify_no_extra_lowering_trace_records(
    source_map: &SourceMapValidation,
    used_lowering_records: &BTreeSet<usize>,
) -> Result<(), VerificationError> {
    let used_functions = used_lowering_records
        .iter()
        .filter_map(|index| source_map.lowering_trace.get(*index))
        .map(|record| record.function.as_str())
        .collect::<BTreeSet<_>>();
    for (index, record) in source_map.lowering_trace.iter().enumerate() {
        if used_lowering_records.contains(&index) {
            continue;
        }
        if !used_functions.contains(record.function.as_str()) {
            continue;
        }
        return Err(VerificationError::TranslationTraceExtraEvent {
            generated_event_id: record.id.clone(),
        });
    }
    Ok(())
}

fn lowering_trace_record_matches(
    generated_event: &GeneratedTraceEvent,
    record: &LoweringTraceRecord,
) -> bool {
    let anchor = &generated_event.source_map_anchor;
    record.core_event_id == generated_event.core_event_id
        && record.kind == generated_event.kind.as_str()
        && record.binding == generated_event.binding
        && record.order == generated_event.order
        && record.source_map_entry_id == anchor.id
        && record.rs_line == anchor.generated_span.line
        && record.rs_start == anchor.generated_span.start
        && record.rs_end == anchor.generated_span.end
        && record.kobo_start == anchor.kobo_span.start
        && record.kobo_end == anchor.kobo_span.end
        && record.lowering_phase == generated_event.lowering_phase
        && record.template_id == generated_event.template_id
        && record.template_version == generated_event.template_version
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
