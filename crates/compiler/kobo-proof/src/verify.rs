use crate::{
    certificate_material_hash, core_material_hash, stable_hash, ArtifactKind, ProofCertificate,
    VerificationError, PROOF_CERTIFICATE_SCHEMA_VERSION, PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA,
    PROOF_TARGET_VERSION,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationContext {
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationReport {
    pub source_hash: String,
    pub core_hash: String,
    pub checked_obligation_events: usize,
    pub certificate_material_hash: String,
}

pub fn parse_certificate_json(source: &str) -> Result<ProofCertificate, VerificationError> {
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
    crate::obligation::verify_cfg_edge_transitions(certificate)?;
    let checked_obligation_events = crate::obligation::replay_obligation_events(certificate)?;
    let certificate_hash = verify_certificate_hash(certificate)?;
    Ok(VerificationReport {
        source_hash: certificate.source.hash.clone(),
        core_hash: certificate.core.hash.clone(),
        checked_obligation_events,
        certificate_material_hash: certificate_hash,
    })
}

fn verify_certificate_header(certificate: &ProofCertificate) -> Result<(), VerificationError> {
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

fn classify_parse_error(error: serde_json::Error) -> VerificationError {
    let message = error.to_string();
    if message.contains("unknown field") {
        return VerificationError::UnknownField { message };
    }
    if message.contains("unknown variant") {
        if message.contains("typed")
            || message.contains("model")
            || message.contains("record")
            || message.contains("opaque")
            || message.contains("debt")
            || message.contains("mystery")
        {
            return VerificationError::UnknownBoundaryPolicy { message };
        }
        return VerificationError::UnknownEventKind { message };
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
