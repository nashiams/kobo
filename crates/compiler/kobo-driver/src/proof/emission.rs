use std::path::Path;

use crate::config::EcosystemAdapterPolicy;
use kobo_codegen::KoboSourceMap;
use kobo_ir::{lower_core_program, ScenarioProgram};
use kobo_proof::{
    core_material_hash, derive_translation_validation, AdapterEvidence, ArtifactKind,
    AsyncModelEvidence, BoundaryAssumption, CandidateAdmissionEvidence, CoreCfgEdge, CoreCfgNode,
    CoreLoopBackEdgeFact, CoreLoopExitFact, CoreTraceEvent, CoverageLoss, FunctionSummary,
    GeneratedTraceEvent, HashEvidence, LoopInvariantEvidence, ObligationEvent, ObligationState,
    OpaqueLedgerEntry, ProofCertificate, ReplayGrade, TranslationValidationInput,
};

use super::async_model::async_model_evidence;
use super::boundaries::{
    adapter_adjusted_replay_grade, adapter_evidence, boundary_evidence,
    runtime_boundary_adapter_evidence, sort_adapter_evidence, RuntimeBoundaryEvidence,
};
use super::bounded::bounded_evidence;
use super::candidates::candidate_admission_evidence;
use super::certificate::certificate_with_material_hash;
use super::core_cfg::{
    core_cfg_edges, core_cfg_nodes, core_loop_exit_facts, core_loop_facts, loop_labels_by_id,
    LoopRegionIndex,
};
use super::loop_invariants::{loop_invariant_evidence, user_loop_invariant_directive};
use super::obligations::{coverage_loss, function_summaries, obligation_evidence};
use super::templates::template_evidence;
use super::traces::{core_trace_evidence, generated_trace_evidence, trace_hashes};
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

pub(super) struct CoreEvidenceParts {
    pub(super) core_program: kobo_ir::CoreProgram,
    pub(super) cfg_nodes: Vec<CoreCfgNode>,
    pub(super) cfg_edges: Vec<CoreCfgEdge>,
    pub(super) loop_facts: Vec<CoreLoopBackEdgeFact>,
    pub(super) loop_exit_facts: Vec<CoreLoopExitFact>,
    pub(super) loop_regions: LoopRegionIndex,
    pub(super) async_model: AsyncModelEvidence,
    pub(super) hash: String,
}

pub(super) struct BoundaryEvidenceParts {
    pub(super) hashes: Vec<HashEvidence>,
    pub(super) assumptions: Vec<BoundaryAssumption>,
    pub(super) opaque_ledger: Vec<OpaqueLedgerEntry>,
}

pub(super) struct ObligationEvidenceParts {
    pub(super) entry_env: Vec<ObligationState>,
    pub(super) exit_env: Vec<ObligationState>,
    pub(super) events: Vec<ObligationEvent>,
    pub(super) function_summaries: Vec<FunctionSummary>,
    pub(super) coverage_loss: Vec<CoverageLoss>,
}

pub(super) struct AdapterEvidenceParts {
    pub(super) replay_grade: ReplayGrade,
    pub(super) confidence: Vec<AdapterEvidence>,
    pub(super) candidate_admission: Vec<CandidateAdmissionEvidence>,
}

pub(super) struct TraceEvidenceParts {
    pub(super) core_trace: Vec<CoreTraceEvent>,
    pub(super) generated_trace: Vec<GeneratedTraceEvent>,
    pub(super) hashes: Vec<HashEvidence>,
    pub(super) translation_validation: kobo_proof::TranslationValidationEvidence,
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

    certificate_with_material_hash(
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
    )
    .map_err(ProofEmissionError::Serialize)
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
