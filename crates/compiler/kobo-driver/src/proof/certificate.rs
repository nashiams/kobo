use kobo_proof::{
    certificate_material_hash, stable_hash, BoundedProofEvidence, CoreEvidence, HashEvidence,
    LoopInvariantEvidence, ProofCertificate, SourceEvidence, TemplateSchemaEvidence,
    PROOF_CERTIFICATE_SCHEMA_VERSION, PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA,
    PROOF_TARGET_VERSION,
};

use super::emission::{
    AdapterEvidenceParts, BoundaryEvidenceParts, CoreEvidenceParts, ObligationEvidenceParts,
    ProofEmissionInput, TraceEvidenceParts,
};

pub(super) fn certificate_with_material_hash(
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
) -> Result<ProofCertificate, serde_json::Error> {
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
