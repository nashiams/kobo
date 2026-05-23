use std::collections::{BTreeMap, BTreeSet};

use crate::{
    certificate_material_hash, core_material_hash, stable_hash, AdapterConfidence,
    BoundaryAssumption, BoundaryPolicy, ObligationEvent, ObligationEventKind, ObligationState,
    ProofCertificate, ReplayGrade, TemplateVersionEvidence, VerificationError,
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
    verify_source_hash(certificate, context)?;
    verify_core_hash(certificate)?;
    verify_cancel_edges(certificate)?;
    verify_async_model(certificate)?;
    verify_template_versions(certificate)?;
    verify_template_hashes(certificate)?;
    verify_boundary_policies(certificate)?;
    verify_adapter_confidence(certificate)?;
    verify_boundary_hashes(certificate)?;
    let checked_obligation_events = replay_obligation_events(certificate)?;
    let certificate_hash = verify_certificate_hash(certificate)?;
    Ok(VerificationReport {
        source_hash: certificate.source.hash.clone(),
        core_hash: certificate.core.hash.clone(),
        checked_obligation_events,
        certificate_material_hash: certificate_hash,
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

fn verify_cancel_edges(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let mut await_blocks = BTreeSet::<String>::new();
    let mut cancel_blocks = BTreeSet::<String>::new();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await" {
            await_blocks.insert(edge.from.clone());
            if edge.to == "await_cancel" {
                cancel_blocks.insert(edge.from.clone());
            }
        }
    }
    for block in await_blocks {
        if !cancel_blocks.contains(&block) {
            return Err(VerificationError::MissingCancelEdge { block });
        }
    }
    Ok(())
}

fn verify_async_model(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let cancel_evidence = certificate
        .core
        .async_model
        .cancel_edges
        .iter()
        .map(|edge| (edge.from.as_str(), edge.to.as_str()))
        .collect::<BTreeSet<_>>();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await"
            && edge.to == "await_cancel"
            && !cancel_evidence.contains(&(edge.from.as_str(), "await_cancel"))
        {
            return Err(VerificationError::MissingAsyncCancelEvidence {
                block: edge.from.clone(),
            });
        }
    }

    let suspension_blocks = certificate
        .core
        .async_model
        .suspension_states
        .iter()
        .map(|state| state.block.as_str())
        .collect::<BTreeSet<_>>();
    for edge in &certificate.core.cfg_edges {
        if edge.kind == "await" && !suspension_blocks.contains(edge.from.as_str()) {
            return Err(VerificationError::MissingAsyncCancelEvidence {
                block: edge.from.clone(),
            });
        }
    }

    Ok(())
}

fn verify_template_versions(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    for template in &certificate.template_versions {
        if !is_supported_template_version(template) {
            return Err(VerificationError::StaleTemplateVersion {
                id: template.id.clone(),
                version: template.version.clone(),
            });
        }
    }
    Ok(())
}

fn is_supported_template_version(template: &TemplateVersionEvidence) -> bool {
    matches!(template.version.as_str(), "v0.13.0")
}

fn verify_template_hashes(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    let recorded_hashes = certificate
        .template_hashes
        .iter()
        .map(|hash| (hash.id.as_str(), hash.hash.as_str()))
        .collect::<BTreeMap<_, _>>();
    for template in &certificate.template_versions {
        let observed = serde_json::to_string(template)
            .map(|material| stable_hash(&material))
            .map_err(|error| VerificationError::Parse {
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

fn verify_boundary_policies(certificate: &ProofCertificate) -> Result<(), VerificationError> {
    verify_opaque_edges_have_ledger(certificate)?;
    verify_exact_replay_boundaries(certificate)
}

fn verify_adapter_confidence(certificate: &ProofCertificate) -> Result<(), VerificationError> {
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

fn verify_opaque_edges_have_ledger(
    certificate: &ProofCertificate,
) -> Result<(), VerificationError> {
    let ledger_boundaries = certificate
        .opaque_edge_ledger
        .iter()
        .map(|entry| entry.boundary.as_str())
        .collect::<BTreeSet<_>>();
    for assumption in &certificate.boundary_assumptions {
        if assumption.policy == BoundaryPolicy::Opaque
            && !ledger_boundaries.contains(assumption.boundary.as_str())
        {
            return Err(VerificationError::OpaqueEdgeWithoutLedger {
                boundary: assumption.boundary.clone(),
            });
        }
    }
    Ok(())
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
    Ok(())
}

fn exact_replay_disallows(policy: BoundaryPolicy) -> bool {
    matches!(
        policy,
        BoundaryPolicy::Opaque | BoundaryPolicy::Outside | BoundaryPolicy::Debt
    )
}

fn verify_boundary_hashes(certificate: &ProofCertificate) -> Result<(), VerificationError> {
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

fn boundary_hash(assumption: &BoundaryAssumption) -> Result<String, VerificationError> {
    serde_json::to_string(assumption)
        .map(|material| stable_hash(&material))
        .map_err(|error| VerificationError::Parse {
            message: error.to_string(),
        })
}

fn replay_obligation_events(certificate: &ProofCertificate) -> Result<usize, VerificationError> {
    let mut env = BTreeMap::<String, String>::new();
    for event in &certificate.obligation_events {
        apply_obligation_event(event, &mut env);
        verify_event_state_after(event, &env)?;
    }
    verify_exit_env(certificate, &env)?;
    reject_unresolved_exit(&env)?;
    Ok(certificate.obligation_events.len())
}

fn apply_obligation_event(event: &ObligationEvent, env: &mut BTreeMap<String, String>) {
    match event.kind {
        ObligationEventKind::Create => {
            if let Some(binding) = event.binding.as_ref() {
                env.insert(binding.clone(), "owned".to_owned());
            }
        }
        ObligationEventKind::Discharge => {
            if let Some(binding) = event.binding.as_ref() {
                env.insert(binding.clone(), "resolved".to_owned());
            }
        }
        ObligationEventKind::Transfer => {
            if let Some(binding) = event.binding.as_ref() {
                env.insert(binding.clone(), "transferred".to_owned());
            }
        }
        ObligationEventKind::Move => {
            if let Some(binding) = event.binding.as_ref() {
                env.insert(binding.clone(), "moved".to_owned());
            }
        }
        ObligationEventKind::BranchUnresolved => {
            if let Some(binding) = event.binding.as_ref() {
                env.insert(binding.clone(), "branch_unresolved".to_owned());
            }
        }
        ObligationEventKind::Escape => {
            if let Some(binding) = event.binding.as_ref() {
                env.insert(binding.clone(), "escaped".to_owned());
            }
        }
        ObligationEventKind::UnsupportedContainer | ObligationEventKind::Call => {}
    }
}

fn verify_event_state_after(
    event: &ObligationEvent,
    env: &BTreeMap<String, String>,
) -> Result<(), VerificationError> {
    let expected = event_state_map(&event.state_after);
    for (binding, expected_state) in expected {
        let observed_state = env.get(&binding).cloned().unwrap_or_default();
        if observed_state != expected_state {
            if expected_state == "resolved" && observed_state == "owned" {
                return Err(VerificationError::RemovedDischarge { binding });
            }
            return Err(VerificationError::ObligationReplayMismatch {
                binding,
                expected: expected_state,
                observed: observed_state,
            });
        }
    }
    Ok(())
}

fn verify_exit_env(
    certificate: &ProofCertificate,
    env: &BTreeMap<String, String>,
) -> Result<(), VerificationError> {
    let expected = event_state_map(&certificate.exit_env);
    for (binding, expected_state) in expected {
        let observed_state = env.get(&binding).cloned().unwrap_or_default();
        if observed_state != expected_state {
            if expected_state == "resolved" && observed_state == "owned" {
                return Err(VerificationError::RemovedDischarge { binding });
            }
            return Err(VerificationError::ObligationReplayMismatch {
                binding,
                expected: expected_state,
                observed: observed_state,
            });
        }
    }
    Ok(())
}

fn reject_unresolved_exit(env: &BTreeMap<String, String>) -> Result<(), VerificationError> {
    for (binding, state) in env {
        if matches!(state.as_str(), "owned" | "moved" | "branch_unresolved") {
            return Err(VerificationError::UnresolvedExitObligation {
                binding: binding.clone(),
            });
        }
    }
    Ok(())
}

fn event_state_map(states: &[ObligationState]) -> BTreeMap<String, String> {
    states
        .iter()
        .map(|state| (state.binding.clone(), state.state.clone()))
        .collect()
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
