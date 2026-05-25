use std::collections::{BTreeMap, BTreeSet};

use crate::{
    stable_hash, template_schema_hash, BoundaryAssumption, BoundaryPolicy, ProofCertificate,
    ReplayGrade, TemplateSchemaEvidence, VerificationError,
};

pub(crate) fn verify_template_schemas(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    for template in &certificate.template_schemas {
        if !is_supported_template_schema(template) {
            return Err(VerificationError::UnsupportedTemplateSchema {
                id: template.id.clone(),
                template_schema: template.template_schema.clone(),
                schema_version: template.schema_version,
            });
        }
    }
    Ok(())
}

pub(crate) fn verify_template_hashes(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let recorded_hashes = certificate
        .template_hashes
        .iter()
        .map(|hash| (hash.id.as_str(), hash.hash.as_str()))
        .collect::<BTreeMap<_, _>>();
    for template in &certificate.template_schemas {
        let observed =
            template_schema_hash(template).map_err(|error| VerificationError::Parse {
                message: error.to_string(),
            })?;
        let expected = recorded_hashes
            .get(template.id.as_str())
            .copied()
            .unwrap_or("");
        if expected != observed {
            return Err(VerificationError::TemplateHashMismatch {
                id: template.id.clone(),
                expected: expected.to_owned(),
                observed,
            });
        }
    }
    Ok(())
}

pub(crate) fn verify_boundary_policies(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    verify_opaque_edges_have_ledger(certificate)?;
    verify_exact_replay_boundaries(certificate)
}

pub(crate) fn verify_boundary_hashes(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let recorded_hashes = certificate
        .boundary_assumption_hashes
        .iter()
        .map(|hash| (hash.id.as_str(), hash.hash.as_str()))
        .collect::<BTreeMap<_, _>>();
    for assumption in &certificate.boundary_assumptions {
        let Some(expected) = recorded_hashes.get(assumption.id.as_str()).copied() else {
            continue;
        };
        let observed = boundary_hash(assumption)?;
        if expected != observed {
            return Err(VerificationError::BoundaryHashMismatch {
                id: assumption.id.clone(),
                expected: expected.to_owned(),
                observed,
            });
        }
    }
    Ok(())
}

fn is_supported_template_schema(template: &TemplateSchemaEvidence) -> bool {
    template.template_schema == "lifecycle-template"
        && template.schema_version == 1
        && matches!(
            template.source.as_str(),
            "built_in" | "declaration" | "adapter" | "summary"
        )
        && !matches!(
            template.confidence.as_str(),
            "metadata-only" | "metadata_only" | "sampled" | "stale"
        )
        && !template.lifecycle_owner.trim().is_empty()
        && !template.cancel_policy.trim().is_empty()
        && !template.registry_source.trim().is_empty()
}

fn verify_opaque_edges_have_ledger(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let opaque_cfg_edges = certificate
        .core
        .cfg_edges
        .iter()
        .filter(|edge| is_opaque_boundary_edge(&edge.kind, &edge.to))
        .map(|edge| edge.id.as_str())
        .collect::<BTreeSet<_>>();
    let cfg_backed_ledgers = certificate
        .opaque_edge_ledger
        .iter()
        .filter(|entry| opaque_cfg_edges.contains(entry.edge_id.as_str()))
        .map(|entry| entry.boundary.as_str())
        .collect::<BTreeSet<_>>();
    for ledger in &certificate.opaque_edge_ledger {
        if !opaque_cfg_edges.contains(ledger.edge_id.as_str()) {
            return Err(VerificationError::OpaqueEdgeWithoutLedger {
                boundary: ledger.boundary.clone(),
            });
        }
    }
    for assumption in &certificate.boundary_assumptions {
        if assumption.policy == BoundaryPolicy::Opaque
            && !cfg_backed_ledgers.contains(assumption.boundary.as_str())
        {
            return Err(VerificationError::OpaqueEdgeWithoutLedger {
                boundary: assumption.boundary.clone(),
            });
        }
    }
    Ok(())
}

fn is_opaque_boundary_edge(kind: &str, target: &str) -> bool {
    kind == "opaque_boundary" || target == "opaque_boundary"
}

fn verify_exact_replay_boundaries(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    if certificate.replay_grade != ReplayGrade::Exact {
        return Ok(());
    }
    for assumption in &certificate.boundary_assumptions {
        if exact_replay_disallows(assumption.policy.clone()) {
            return Err(VerificationError::ExactReplayWithDebtBoundary {
                boundary: assumption.boundary.clone(),
                policy: boundary_policy_name(&assumption.policy).to_owned(),
            });
        }
    }
    if let Some(loss) = certificate.coverage_loss.first() {
        return Err(VerificationError::ExactReplayWithCoverageLoss {
            kind: loss.kind.clone(),
            label: loss.label.clone(),
        });
    }
    Ok(())
}

fn exact_replay_disallows(policy: BoundaryPolicy) -> bool {
    matches!(
        policy,
        BoundaryPolicy::Opaque
            | BoundaryPolicy::Outside
            | BoundaryPolicy::Debt
            | BoundaryPolicy::Stub
    )
}

fn boundary_hash(assumption: &BoundaryAssumption) -> Result<String, VerificationError> {
    serde_json::to_string(assumption)
        .map(|material| stable_hash(&material))
        .map_err(|error| VerificationError::Parse {
            message: error.to_string(),
        })
}

fn boundary_policy_name(policy: &BoundaryPolicy) -> &'static str {
    match policy {
        BoundaryPolicy::Typed => "typed",
        BoundaryPolicy::Model => "model",
        BoundaryPolicy::Record => "record",
        BoundaryPolicy::Activity => "activity",
        BoundaryPolicy::Stub => "stub",
        BoundaryPolicy::Outside => "outside",
        BoundaryPolicy::Opaque => "opaque",
        BoundaryPolicy::Debt => "debt",
        BoundaryPolicy::Unselected => "unselected",
    }
}
