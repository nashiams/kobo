use super::{
    adapter_adjusted_replay_grade, adapter_evidence, async_model_evidence, boundary_evidence,
    bounded_evidence, candidate_admission_evidence, certificate_material_hash, core_cfg_edges,
    core_cfg_nodes, core_loop_exit_facts, core_loop_facts, core_material_hash, core_trace_evidence,
    coverage_loss, derive_translation_validation, function_summaries, generated_trace_evidence,
    loop_invariant_evidence, loop_labels_by_id, lower_core_program, obligation_evidence,
    runtime_boundary_adapter_evidence, sort_adapter_evidence, stable_hash, template_evidence,
    trace_hashes, user_loop_invariant_directive, AdapterEvidence, ArtifactKind, AsyncModelEvidence,
    BoundaryAssumption, BoundedProofEvidence, CandidateAdmissionEvidence, CoreCfgEdge, CoreCfgNode,
    CoreEvidence, CoreLoopBackEdgeFact, CoreLoopExitFact, CoreTraceEvent, CoverageLoss,
    EcosystemAdapterPolicy, FunctionSummary, GeneratedTraceEvent, HashEvidence, KoboSourceMap,
    LoopInvariantEvidence, LoopRegionIndex, ObligationEvent, ObligationState, OpaqueLedgerEntry,
    Path, ProofCertificate, ReplayGrade, RuntimeBoundaryEvidence, ScenarioProgram, SourceEvidence,
    TemplateSchemaEvidence, TranslationValidationInput, PROOF_CERTIFICATE_SCHEMA_VERSION,
    PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA, PROOF_TARGET_VERSION,
};
pub struct ProofEmissionInput<'a> {
    pub source_path: &'a Path,
    pub source: &'a str,
    pub program: &'a ScenarioProgram,
    pub adapter_policies: &'a [EcosystemAdapterPolicy],
    pub runtime_boundaries: &'a [RuntimeBoundaryEvidence],
    pub replay_grade: ReplayGrade,
    pub artifact_kind: ArtifactKind,
    pub source_map: Option<&'a KoboSourceMap>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProofEmissionError {
    #[error("failed to serialize proof material: {0}")]
    Serialize(#[from] serde_json::Error),
}

struct CoreEvidenceParts {
    core_program: kobo_ir::CoreProgram,
    cfg_nodes: Vec<CoreCfgNode>,
    cfg_edges: Vec<CoreCfgEdge>,
    loop_facts: Vec<CoreLoopBackEdgeFact>,
    loop_exit_facts: Vec<CoreLoopExitFact>,
    loop_regions: LoopRegionIndex,
    async_model: AsyncModelEvidence,
    hash: String,
}

struct BoundaryEvidenceParts {
    hashes: Vec<HashEvidence>,
    assumptions: Vec<BoundaryAssumption>,
    opaque_ledger: Vec<OpaqueLedgerEntry>,
}

struct ObligationEvidenceParts {
    entry_env: Vec<ObligationState>,
    exit_env: Vec<ObligationState>,
    events: Vec<ObligationEvent>,
    function_summaries: Vec<FunctionSummary>,
    coverage_loss: Vec<CoverageLoss>,
}

struct AdapterEvidenceParts {
    replay_grade: ReplayGrade,
    confidence: Vec<AdapterEvidence>,
    candidate_admission: Vec<CandidateAdmissionEvidence>,
}

struct TraceEvidenceParts {
    core_trace: Vec<CoreTraceEvent>,
    generated_trace: Vec<GeneratedTraceEvent>,
    hashes: Vec<HashEvidence>,
    translation_validation: kobo_proof::TranslationValidationEvidence,
}

pub fn emit_proof_certificate(
    input: ProofEmissionInput<'_>,
) -> Result<ProofCertificate, ProofEmissionError> {
    let source_path = input.source_path.display().to_string();
    let core = build_core_evidence(&source_path, &input)?;
    let (template_hashes, template_schemas) =
        template_evidence(&source_path, input.source, input.program)?;
    let boundaries = build_boundary_evidence(&source_path, &input, &core.cfg_edges)?;
    let obligations = build_obligation_evidence(&source_path, &input, &core);
    let adapters = build_adapter_evidence(&input);
    let loop_invariants = build_loop_invariants(
        &source_path,
        &input,
        &core,
        &obligations.events,
        &template_hashes,
    );
    let bounded_evidence = bounded_evidence(
        &source_path,
        input.source,
        input.program,
        &core.loop_facts,
        &core.loop_exit_facts,
    );
    let traces = build_trace_evidence(&source_path, &input, &obligations.events)?;

    let mut certificate = assemble_certificate(
        input,
        source_path,
        core,
        boundaries,
        obligations,
        adapters,
        template_hashes,
        template_schemas,
        loop_invariants,
        bounded_evidence,
        traces,
    );
    certificate.certificate_material_hash = certificate_material_hash(&certificate)?;
    Ok(certificate)
}

fn build_core_evidence(
    source_path: &str,
    input: &ProofEmissionInput<'_>,
) -> Result<CoreEvidenceParts, ProofEmissionError> {
    let core_program = lower_core_program(input.program);
    let cfg_nodes = core_cfg_nodes(source_path, input.source, &core_program.functions);
    let loop_labels = loop_labels_by_id(input.program);
    let cfg_edges = core_cfg_edges(
        source_path,
        input.source,
        &core_program.functions,
        &loop_labels,
    );
    let loop_facts = core_loop_facts(&cfg_edges);
    let loop_exit_facts = core_loop_exit_facts(&cfg_edges);
    let loop_regions = LoopRegionIndex::from_core(&core_program.functions, &cfg_edges, &loop_facts);
    let async_model = async_model_evidence(
        source_path,
        input.source,
        input.program,
        &core_program.functions,
    );
    let core_hash = core_material_hash(
        core_program.core_version,
        &cfg_nodes,
        &cfg_edges,
        &loop_facts,
        &loop_exit_facts,
        &async_model,
    )?;

    Ok(CoreEvidenceParts {
        core_program,
        cfg_nodes,
        cfg_edges,
        loop_facts,
        loop_exit_facts,
        loop_regions,
        async_model,
        hash: core_hash,
    })
}

fn build_boundary_evidence(
    source_path: &str,
    input: &ProofEmissionInput<'_>,
    cfg_edges: &[CoreCfgEdge],
) -> Result<BoundaryEvidenceParts, ProofEmissionError> {
    let (hashes, assumptions, opaque_ledger) =
        boundary_evidence(source_path, input.source, input.program, cfg_edges)?;
    Ok(BoundaryEvidenceParts {
        hashes,
        assumptions,
        opaque_ledger,
    })
}

fn build_obligation_evidence(
    source_path: &str,
    input: &ProofEmissionInput<'_>,
    core: &CoreEvidenceParts,
) -> ObligationEvidenceParts {
    let (entry_env, exit_env, events) = obligation_evidence(
        source_path,
        input.source,
        &core.core_program.functions,
        &core.loop_regions,
    );
    let function_summaries = function_summaries(input.program, &entry_env, &exit_env, events.len());
    let coverage_loss = coverage_loss(input.program);

    ObligationEvidenceParts {
        entry_env,
        exit_env,
        events,
        function_summaries,
        coverage_loss,
    }
}

fn build_adapter_evidence(input: &ProofEmissionInput<'_>) -> AdapterEvidenceParts {
    let mut confidence = adapter_evidence(
        input.program,
        input.adapter_policies,
        input.replay_grade.clone(),
    );
    confidence.extend(runtime_boundary_adapter_evidence(
        input.runtime_boundaries,
        input.replay_grade.clone(),
    ));
    sort_adapter_evidence(&mut confidence);
    let replay_grade = adapter_adjusted_replay_grade(input.replay_grade.clone(), &confidence);
    for adapter in &mut confidence {
        adapter.replay_grade = replay_grade.clone();
    }
    let candidate_admission = candidate_admission_evidence(
        input.source_path,
        input.source,
        replay_grade.clone(),
        &confidence,
    );

    AdapterEvidenceParts {
        replay_grade,
        confidence,
        candidate_admission,
    }
}

fn build_loop_invariants(
    source_path: &str,
    input: &ProofEmissionInput<'_>,
    core: &CoreEvidenceParts,
    obligation_events: &[ObligationEvent],
    template_hashes: &[HashEvidence],
) -> Vec<LoopInvariantEvidence> {
    let user_invariant = user_loop_invariant_directive(source_path, input.source, input.program);
    loop_invariant_evidence(
        input.program,
        &core.core_program.functions,
        &core.loop_facts,
        obligation_events,
        template_hashes,
        user_invariant.as_ref(),
    )
}

fn build_trace_evidence(
    source_path: &str,
    input: &ProofEmissionInput<'_>,
    obligation_events: &[ObligationEvent],
) -> Result<TraceEvidenceParts, ProofEmissionError> {
    let core_trace =
        core_trace_evidence(input.program, obligation_events, source_path, input.source);
    let generated_trace = generated_trace_evidence(
        source_path,
        input.source,
        input.program,
        &core_trace,
        input.source_map,
    );
    let translation_validation = derive_translation_validation(TranslationValidationInput {
        core_trace: &core_trace,
        generated_trace: &generated_trace,
        has_source_map: input.source_map.is_some(),
    });
    let hashes = trace_hashes(&core_trace, &generated_trace)?;

    Ok(TraceEvidenceParts {
        core_trace,
        generated_trace,
        hashes,
        translation_validation,
    })
}

fn assemble_certificate(
    input: ProofEmissionInput<'_>,
    source_path: String,
    core: CoreEvidenceParts,
    boundaries: BoundaryEvidenceParts,
    obligations: ObligationEvidenceParts,
    adapters: AdapterEvidenceParts,
    template_hashes: Vec<HashEvidence>,
    template_schemas: Vec<TemplateSchemaEvidence>,
    loop_invariants: Vec<LoopInvariantEvidence>,
    bounded_evidence: Vec<BoundedProofEvidence>,
    traces: TraceEvidenceParts,
) -> ProofCertificate {
    ProofCertificate {
        schema_version: PROOF_CERTIFICATE_SCHEMA_VERSION,
        proof_target_version: PROOF_TARGET_VERSION.to_owned(),
        semantic_schema: PROOF_SEMANTIC_SCHEMA.to_owned(),
        artifact_kind: input.artifact_kind,
        claim_scope: PROOF_CLAIM_SCOPE.to_owned(),
        compiler_version: env!("CARGO_PKG_VERSION").to_owned(),
        source: SourceEvidence {
            path: source_path,
            hash: stable_hash(input.source),
        },
        core: CoreEvidence {
            hash: core.hash,
            version: core.core_program.core_version.to_owned(),
            cfg_nodes: core.cfg_nodes,
            cfg_edges: core.cfg_edges,
            loop_facts: core.loop_facts,
            loop_exit_facts: core.loop_exit_facts,
            async_model: core.async_model,
        },
        replay_grade: adapters.replay_grade,
        template_hashes,
        template_schemas,
        boundary_assumption_hashes: boundaries.hashes,
        boundary_assumptions: boundaries.assumptions,
        adapter_confidence: adapters.confidence,
        obligation_events: obligations.events,
        entry_env: obligations.entry_env,
        exit_env: obligations.exit_env,
        function_summaries: obligations.function_summaries,
        coverage_loss: obligations.coverage_loss,
        loop_invariants,
        bounded_evidence,
        core_obligation_trace: traces.core_trace,
        generated_rust_trace: traces.generated_trace,
        trace_hashes: traces.hashes,
        translation_validation: traces.translation_validation,
        opaque_edge_ledger: boundaries.opaque_ledger,
        candidate_admission: adapters.candidate_admission,
        certificate_material_hash: String::new(),
    }
}
