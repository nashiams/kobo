use crate::{
    certificate_material_hash, core_material_hash, stable_hash, ArtifactKind, ProofCertificate,
    VerificationError, PROOF_CERTIFICATE_SCHEMA_VERSION, PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA,
    PROOF_TARGET_VERSION,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationContext {
    pub source: String,
    pub source_map: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationReport {
    pub source_hash: String,
    pub core_hash: String,
    pub checked_obligation_events: usize,
    pub certificate_material_hash: String,
    pub translation_validation_status: String,
    pub bounded_wording: Vec<String>,
}

pub fn parse_certificate_json(source: &str) -> Result<ProofCertificate, VerificationError> {
    preflight_certificate_json(source)?;
    serde_json::from_str::<ProofCertificate>(source).map_err(classify_parse_error)
}

pub fn verify_certificate(
    certificate: &ProofCertificate,
    context: &VerificationContext,
) -> Result<VerificationReport, VerificationError> {
    verify_certificate_header(certificate)?;
    verify_source_hash(certificate, context)?;
    verify_core_hash(certificate)?;
    crate::async_model::verify_cancel_edges(certificate)?;
    crate::async_model::verify_async_model(certificate, &context.source)?;
    crate::boundary::verify_template_schemas(certificate)?;
    crate::boundary::verify_template_hashes(certificate)?;
    crate::boundary::verify_boundary_policies(certificate)?;
    crate::adapter::verify_adapter_confidence(certificate)?;
    crate::candidate::verify_candidate_admission(certificate)?;
    crate::boundary::verify_boundary_hashes(certificate)?;
    crate::invariant::verify_loop_invariants(certificate)?;
    crate::bounded::verify_bounded_evidence(certificate)?;
    crate::translation::verify_translation_validation(certificate, context.source_map.as_deref())?;
    crate::obligation::verify_cfg_edge_transitions(certificate)?;
    let checked_obligation_events = crate::obligation::replay_obligation_events(certificate)?;
    let certificate_hash = verify_certificate_hash(certificate)?;
    Ok(VerificationReport {
        source_hash: certificate.source.hash.clone(),
        core_hash: certificate.core.hash.clone(),
        checked_obligation_events,
        certificate_material_hash: certificate_hash,
        translation_validation_status: certificate
            .translation_validation
            .status
            .as_str()
            .to_owned(),
        bounded_wording: certificate
            .bounded_evidence
            .iter()
            .map(|evidence| evidence.wording.clone())
            .collect(),
    })
}

pub fn verify_certificate_header(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    verify_header_field(
        "schema_version",
        PROOF_CERTIFICATE_SCHEMA_VERSION.to_string(),
        certificate.schema_version.to_string(),
    )?;
    verify_header_field(
        "proof_target_version",
        PROOF_TARGET_VERSION.to_owned(),
        certificate.proof_target_version.clone(),
    )?;
    verify_header_field(
        "semantic_schema",
        PROOF_SEMANTIC_SCHEMA.to_owned(),
        certificate.semantic_schema.clone(),
    )?;
    verify_header_field(
        "claim_scope",
        PROOF_CLAIM_SCOPE.to_owned(),
        certificate.claim_scope.clone(),
    )?;
    match certificate.artifact_kind {
        ArtifactKind::Kproof | ArtifactKind::KwitProofJson => Ok(()),
    }
}

fn preflight_certificate_json(source: &str) -> Result<(), VerificationError> {
    let value: serde_json::Value =
        serde_json::from_str(source).map_err(|error| VerificationError::Parse {
            message: error.to_string(),
        })?;
    if let Some(kind) = value.get("artifact_kind") {
        validate_header_enum(
            "artifact_kind",
            kind,
            &["kproof", "kwit.proof.json"],
            "kproof or kwit.proof.json",
        )?;
    }
    if let Some(replay_grade) = value.get("replay_grade") {
        validate_replay_grade("replay_grade", replay_grade)?;
    }
    validate_boundary_policy_fields(&value)?;
    validate_obligation_event_kind_fields(&value)?;
    validate_obligation_status_fields(&value)?;
    validate_adapter_evidence_fields("adapter_confidence", value.get("adapter_confidence"))?;
    validate_candidate_admission_fields(&value)?;
    Ok(())
}

fn verify_header_field(
    field: &'static str,
    expected: String,
    observed: String,
) -> Result<(), VerificationError> {
    if observed == expected {
        return Ok(());
    }
    Err(VerificationError::UnsupportedCertificateHeader {
        field: field.to_owned(),
        expected,
        observed,
    })
}

fn validate_boundary_policy_fields(value: &serde_json::Value) -> Result<(), VerificationError> {
    let Some(boundaries) = value
        .get("boundary_assumptions")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(());
    };
    for (index, boundary) in boundaries.iter().enumerate() {
        if let Some(policy) = boundary.get("policy") {
            let field = format!("boundary_assumptions[{index}].policy");
            validate_boundary_policy(&field, policy)?;
        }
    }
    Ok(())
}

fn validate_obligation_event_kind_fields(
    value: &serde_json::Value,
) -> Result<(), VerificationError> {
    let Some(events) = value
        .get("obligation_events")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(());
    };
    for (index, event) in events.iter().enumerate() {
        if let Some(kind) = event.get("kind") {
            let field = format!("obligation_events[{index}].kind");
            validate_obligation_event_kind(&field, kind)?;
        }
    }
    Ok(())
}

fn validate_obligation_status_fields(value: &serde_json::Value) -> Result<(), VerificationError> {
    validate_obligation_state_array("entry_env", value.get("entry_env"))?;
    validate_obligation_state_array("exit_env", value.get("exit_env"))?;

    if let Some(events) = value
        .get("obligation_events")
        .and_then(serde_json::Value::as_array)
    {
        for (index, event) in events.iter().enumerate() {
            validate_obligation_state_array(
                &format!("obligation_events[{index}].state_before"),
                event.get("state_before"),
            )?;
            validate_obligation_state_array(
                &format!("obligation_events[{index}].state_after"),
                event.get("state_after"),
            )?;
        }
    }

    if let Some(async_model) = value.get("core").and_then(|core| core.get("async_model")) {
        validate_future_state_obligations(
            "core.async_model.future_state_obligations",
            async_model.get("future_state_obligations"),
        )?;
        validate_select_path_states(
            "core.async_model.select_paths",
            async_model.get("select_paths"),
        )?;
    }

    if let Some(summaries) = value
        .get("function_summaries")
        .and_then(serde_json::Value::as_array)
    {
        for (index, summary) in summaries.iter().enumerate() {
            validate_obligation_state_array(
                &format!("function_summaries[{index}].entry_env"),
                summary.get("entry_env"),
            )?;
            validate_obligation_state_array(
                &format!("function_summaries[{index}].exit_env"),
                summary.get("exit_env"),
            )?;
        }
    }

    Ok(())
}

fn validate_future_state_obligations(
    field_prefix: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), VerificationError> {
    let Some(states) = value.and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for (index, state) in states.iter().enumerate() {
        if let Some(status) = state.get("state") {
            validate_obligation_status(&format!("{field_prefix}[{index}].state"), status)?;
        }
    }
    Ok(())
}

fn validate_select_path_states(
    field_prefix: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), VerificationError> {
    let Some(paths) = value.and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for (index, path) in paths.iter().enumerate() {
        validate_obligation_state_array(
            &format!("{field_prefix}[{index}].obligation_results"),
            path.get("obligation_results"),
        )?;
        validate_obligation_state_array(
            &format!("{field_prefix}[{index}].cancelled_obligations"),
            path.get("cancelled_obligations"),
        )?;
    }
    Ok(())
}

fn validate_obligation_state_array(
    field_prefix: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), VerificationError> {
    let Some(states) = value.and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for (index, state) in states.iter().enumerate() {
        if let Some(status) = state.get("state") {
            validate_obligation_status(&format!("{field_prefix}[{index}].state"), status)?;
        }
    }
    Ok(())
}

fn validate_candidate_admission_fields(value: &serde_json::Value) -> Result<(), VerificationError> {
    let Some(candidates) = value
        .get("candidate_admission")
        .and_then(serde_json::Value::as_array)
    else {
        return Ok(());
    };
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(replay_grade) = candidate.get("replay_grade") {
            validate_optional_replay_grade(
                &format!("candidate_admission[{index}].replay_grade"),
                replay_grade,
            )?;
        }
        validate_adapter_evidence_fields(
            &format!("candidate_admission[{index}].adapter_confidence"),
            candidate.get("adapter_confidence"),
        )?;
    }
    Ok(())
}

fn validate_adapter_evidence_fields(
    field_prefix: &str,
    value: Option<&serde_json::Value>,
) -> Result<(), VerificationError> {
    let Some(adapters) = value.and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for (index, adapter) in adapters.iter().enumerate() {
        if let Some(confidence) = adapter.get("confidence") {
            validate_certificate_enum(
                &format!("{field_prefix}[{index}].confidence"),
                confidence,
                &["exact", "modeled", "sampled", "metadata-only"],
                "exact, modeled, sampled, or metadata-only",
            )?;
        }
        if let Some(replay_grade) = adapter.get("replay_grade") {
            validate_replay_grade(
                &format!("{field_prefix}[{index}].replay_grade"),
                replay_grade,
            )?;
        }
    }
    Ok(())
}

fn validate_replay_grade(field: &str, value: &serde_json::Value) -> Result<(), VerificationError> {
    validate_certificate_enum(
        field,
        value,
        &["exact", "partial", "not_replayable", "debt"],
        "exact, partial, not_replayable, or debt",
    )
}

fn validate_optional_replay_grade(
    field: &str,
    value: &serde_json::Value,
) -> Result<(), VerificationError> {
    if value.is_null() {
        return Ok(());
    }
    validate_replay_grade(field, value)
}

fn validate_boundary_policy(
    field: &str,
    value: &serde_json::Value,
) -> Result<(), VerificationError> {
    let observed = observed_json(value);
    if matches!(
        observed.as_str(),
        "typed"
            | "model"
            | "record"
            | "activity"
            | "stub"
            | "outside"
            | "opaque"
            | "debt"
            | "unselected"
    ) {
        return Ok(());
    }
    Err(VerificationError::UnknownBoundaryPolicy {
        message: format!(
            "{field}: expected typed, model, record, activity, stub, outside, opaque, debt, or unselected; observed {observed}"
        ),
    })
}

fn validate_obligation_event_kind(
    field: &str,
    value: &serde_json::Value,
) -> Result<(), VerificationError> {
    let observed = observed_json(value);
    if matches!(
        observed.as_str(),
        "create"
            | "discharge"
            | "transfer"
            | "move"
            | "branch_unresolved"
            | "escape"
            | "unsupported_container"
            | "call"
    ) {
        return Ok(());
    }
    Err(VerificationError::UnknownEventKind {
        message: format!(
            "{field}: expected create, discharge, transfer, move, branch_unresolved, escape, unsupported_container, or call; observed {observed}"
        ),
    })
}

fn validate_obligation_status(
    field: &str,
    value: &serde_json::Value,
) -> Result<(), VerificationError> {
    validate_certificate_enum(
        field,
        value,
        &[
            "owned",
            "resolved",
            "transferred",
            "moved",
            "branch_unresolved",
            "escaped",
        ],
        "owned, resolved, transferred, moved, branch_unresolved, or escaped",
    )
}

fn validate_header_enum(
    field: &str,
    value: &serde_json::Value,
    allowed: &[&str],
    expected: &str,
) -> Result<(), VerificationError> {
    let observed = observed_json(value);
    if allowed.iter().any(|allowed| *allowed == observed) {
        return Ok(());
    }
    Err(VerificationError::UnsupportedCertificateHeader {
        field: field.to_owned(),
        expected: expected.to_owned(),
        observed,
    })
}

fn validate_certificate_enum(
    field: &str,
    value: &serde_json::Value,
    allowed: &[&str],
    expected: &str,
) -> Result<(), VerificationError> {
    let observed = observed_json(value);
    if allowed.iter().any(|allowed| *allowed == observed) {
        return Ok(());
    }
    Err(VerificationError::UnsupportedCertificateField {
        field: field.to_owned(),
        expected: expected.to_owned(),
        observed,
    })
}

fn observed_json(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn classify_parse_error(error: serde_json::Error) -> VerificationError {
    let message = error.to_string();
    if message.contains("unknown field") {
        return VerificationError::UnknownField { message };
    }
    VerificationError::Parse { message }
}

fn verify_source_hash(
    certificate: &ProofCertificate,
    context: &VerificationContext,
) -> Result<(), VerificationError> {
    if certificate.source.hash.is_empty() {
        return Err(VerificationError::MissingSourceHash);
    }
    let observed = stable_hash(&context.source);
    if observed != certificate.source.hash {
        return Err(VerificationError::SourceHashMismatch {
            expected: certificate.source.hash.clone(),
            observed,
        });
    }
    Ok(())
}

fn verify_core_hash(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let observed = core_material_hash(
        &certificate.core.version,
        &certificate.core.cfg_nodes,
        &certificate.core.cfg_edges,
        &certificate.core.loop_facts,
        &certificate.core.loop_exit_facts,
        &certificate.core.async_model,
    )
    .map_err(|error| VerificationError::Parse {
        message: error.to_string(),
    })?;
    if observed != certificate.core.hash {
        return Err(VerificationError::CoreHashMismatch {
            expected: certificate.core.hash.clone(),
            observed,
        });
    }
    Ok(())
}

fn verify_certificate_hash(certificate: &ProofCertificate) -> Result<String, VerificationError> {
    let observed =
        certificate_material_hash(certificate).map_err(|error| VerificationError::Parse {
            message: error.to_string(),
        })?;
    if observed != certificate.certificate_material_hash {
        return Err(VerificationError::CertificateMaterialHashMismatch {
            expected: certificate.certificate_material_hash.clone(),
            observed,
        });
    }
    Ok(observed)
}
