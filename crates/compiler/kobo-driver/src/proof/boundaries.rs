use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeBoundaryEvidence {
    pub boundary: String,
    pub policy: kobo_ir::ScenarioBoundaryPolicy,
    pub has_recorded_io: bool,
}

pub(super) fn boundary_evidence(
    source_path: &str,
    source: &str,
    program: &ScenarioProgram,
    cfg_edges: &[CoreCfgEdge],
) -> Result<
    (
        Vec<HashEvidence>,
        Vec<BoundaryAssumption>,
        Vec<OpaqueLedgerEntry>,
    ),
    serde_json::Error,
> {
    let mut hashes = Vec::new();
    let mut assumptions = Vec::new();
    let mut opaque_ledger = Vec::new();
    for (index, operation) in program.operations.iter().enumerate() {
        let ScenarioOpKind::ExternalBoundary {
            crate_name,
            policy,
            reason,
            ..
        } = &operation.kind
        else {
            continue;
        };
        let id = format!("boundary-{index}-{crate_name}");
        let assumption = BoundaryAssumption {
            id: id.clone(),
            boundary: crate_name.clone(),
            policy: boundary_policy(policy),
            reason: reason.clone(),
            source_span: source_span_from_kobo(source_path, source, operation.span),
        };
        let hash = serde_json::to_string(&assumption).map(|material| stable_hash(&material))?;
        hashes.push(HashEvidence {
            id: id.clone(),
            hash: hash.clone(),
        });
        if matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Opaque) {
            let edge_id = opaque_cfg_edge_id_for_span(cfg_edges, &assumption.source_span)
                .unwrap_or_else(|| id.clone());
            opaque_ledger.push(OpaqueLedgerEntry {
                edge_id,
                boundary: crate_name.clone(),
                evidence_hash: hash,
            });
        }
        assumptions.push(assumption);
    }
    Ok((hashes, assumptions, opaque_ledger))
}

pub(super) fn opaque_cfg_edge_id_for_span(
    cfg_edges: &[CoreCfgEdge],
    source_span: &SourceSpan,
) -> Option<String> {
    cfg_edges
        .iter()
        .find(|edge| {
            edge.kind == "opaque_boundary"
                && edge.to == "opaque_boundary"
                && edge.source_span == *source_span
        })
        .map(|edge| edge.id.clone())
}

pub fn adapter_evidence(
    program: &ScenarioProgram,
    adapter_policies: &[EcosystemAdapterPolicy],
    requested_replay_grade: ReplayGrade,
) -> Vec<AdapterEvidence> {
    let mut evidence = Vec::new();
    for operation in &program.operations {
        let ScenarioOpKind::ExternalBoundary {
            crate_name, policy, ..
        } = &operation.kind
        else {
            continue;
        };
        let Some(adapter) = adapter_policies
            .iter()
            .find(|adapter| adapter.crate_name == *crate_name)
        else {
            continue;
        };
        let confidence = adapter_confidence(adapter, policy);
        let outcome = adapter_outcome(adapter, &confidence);
        evidence.push(AdapterEvidence {
            boundary: crate_name.clone(),
            adapter: adapter.package.clone(),
            version: adapter.version.clone(),
            confidence,
            replay_grade: requested_replay_grade.clone(),
            outcome,
            reason: adapter
                .reason
                .clone()
                .unwrap_or_else(|| "configured ecosystem adapter".to_owned()),
        });
    }
    sort_adapter_evidence(&mut evidence);
    evidence
}

pub fn runtime_boundary_adapter_evidence(
    boundaries: &[RuntimeBoundaryEvidence],
    requested_replay_grade: ReplayGrade,
) -> Vec<AdapterEvidence> {
    let mut evidence = boundaries
        .iter()
        .filter(|boundary| {
            boundary.has_recorded_io
                && matches!(boundary.policy, kobo_ir::ScenarioBoundaryPolicy::Record)
        })
        .map(|boundary| AdapterEvidence {
            boundary: boundary.boundary.clone(),
            adapter: "recorded-boundary-capture".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            confidence: AdapterConfidence::Exact,
            replay_grade: requested_replay_grade.clone(),
            outcome: "proof".to_owned(),
            reason: "recorded boundary I/O capture".to_owned(),
        })
        .collect::<Vec<_>>();
    sort_adapter_evidence(&mut evidence);
    evidence
}

pub(super) fn sort_adapter_evidence(evidence: &mut Vec<AdapterEvidence>) {
    evidence.sort_by(|left, right| {
        (&left.boundary, &left.adapter, &left.version).cmp(&(
            &right.boundary,
            &right.adapter,
            &right.version,
        ))
    });
    evidence.dedup_by(|left, right| {
        left.boundary == right.boundary
            && left.adapter == right.adapter
            && left.version == right.version
    });
}

pub fn adapter_adjusted_replay_grade(
    requested_replay_grade: ReplayGrade,
    adapters: &[AdapterEvidence],
) -> ReplayGrade {
    if matches!(
        requested_replay_grade,
        ReplayGrade::Debt | ReplayGrade::NotReplayable
    ) {
        return requested_replay_grade;
    }
    if adapters.iter().any(adapter_is_stale) {
        return ReplayGrade::Debt;
    }
    if adapters.iter().any(|adapter| {
        matches!(
            adapter.confidence,
            AdapterConfidence::Sampled | AdapterConfidence::MetadataOnly
        )
    }) {
        return ReplayGrade::NotReplayable;
    }
    if requested_replay_grade == ReplayGrade::Exact
        && adapters
            .iter()
            .any(|adapter| adapter.confidence == AdapterConfidence::Modeled)
    {
        return ReplayGrade::Partial;
    }
    requested_replay_grade
}

pub(super) fn adapter_confidence(
    adapter: &EcosystemAdapterPolicy,
    policy: &kobo_ir::ScenarioBoundaryPolicy,
) -> AdapterConfidence {
    match adapter.confidence.as_deref() {
        Some("exact") => AdapterConfidence::Exact,
        Some("modeled") => AdapterConfidence::Modeled,
        Some("sampled") => AdapterConfidence::Sampled,
        Some("metadata-only") | Some("metadata_only") => AdapterConfidence::MetadataOnly,
        _ if !adapter.validated => AdapterConfidence::MetadataOnly,
        _ if adapter.capture.as_deref() == Some("boundary-io")
            && matches!(policy, kobo_ir::ScenarioBoundaryPolicy::Record) =>
        {
            AdapterConfidence::Exact
        }
        _ if adapter.adapter_runtime.is_some() => AdapterConfidence::Modeled,
        _ => AdapterConfidence::MetadataOnly,
    }
}

pub(super) fn adapter_outcome(
    adapter: &EcosystemAdapterPolicy,
    confidence: &AdapterConfidence,
) -> String {
    if adapter_version_is_stale(adapter.version.as_deref()) {
        return "debt".to_owned();
    }
    match confidence {
        AdapterConfidence::Exact | AdapterConfidence::Modeled => "proof".to_owned(),
        AdapterConfidence::Sampled => "probing_pass".to_owned(),
        AdapterConfidence::MetadataOnly => "metadata_only".to_owned(),
    }
}

pub(super) fn adapter_is_stale(adapter: &AdapterEvidence) -> bool {
    adapter_version_is_stale(adapter.version.as_deref()) || adapter.outcome == "debt"
}

pub(super) fn adapter_version_is_stale(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return true;
    };
    version == "0.0.0" || version.contains("stale")
}

pub(super) fn boundary_policy(policy: &kobo_ir::ScenarioBoundaryPolicy) -> BoundaryPolicy {
    match policy {
        kobo_ir::ScenarioBoundaryPolicy::Typed => BoundaryPolicy::Typed,
        kobo_ir::ScenarioBoundaryPolicy::Model => BoundaryPolicy::Model,
        kobo_ir::ScenarioBoundaryPolicy::Record => BoundaryPolicy::Record,
        kobo_ir::ScenarioBoundaryPolicy::Activity => BoundaryPolicy::Activity,
        kobo_ir::ScenarioBoundaryPolicy::Stub => BoundaryPolicy::Stub,
        kobo_ir::ScenarioBoundaryPolicy::Outside => BoundaryPolicy::Outside,
        kobo_ir::ScenarioBoundaryPolicy::Opaque => BoundaryPolicy::Opaque,
        kobo_ir::ScenarioBoundaryPolicy::Debt => BoundaryPolicy::Debt,
        kobo_ir::ScenarioBoundaryPolicy::Unselected => BoundaryPolicy::Unselected,
    }
}
