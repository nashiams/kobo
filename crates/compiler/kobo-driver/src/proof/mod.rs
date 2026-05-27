use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use crate::config::EcosystemAdapterPolicy;
use kobo_codegen::{KoboSourceMap, LoweringTraceEvent};
use kobo_ir::{
    lower_core_program, CoreBlock, CoreFunction, CoreStatement, CoreStatementKind,
    CoreTerminatorKind, KoboSpan, ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
use kobo_proof::{
    bounded_wording as proof_bounded_wording, certificate_material_hash,
    classify_bounded_completeness, core_material_hash, derive_translation_validation,
    normalized_bound_hash, stable_hash, template_schema_hash, trace_material_hash,
    AdapterConfidence, AdapterEvidence, AsyncModelEvidence, BoundDeclaration, BoundDimension,
    BoundSource, BoundaryAssumption, BoundaryPolicy, BoundedClassificationInput,
    BoundedCompleteness, BoundedHistoryEvidence, BoundedProofEvidence, CancelEdgeEvidence,
    CandidateAdmissionEvidence, CandidateAdmissionFact, CoreCfgEdge, CoreCfgNode, CoreEvidence,
    CoreLoopBackEdgeFact, CoreLoopExitFact, CoreTraceEvent, CoverageLoss, FunctionSummary,
    FutureStateLocalEvidence, FutureStateObligationEvidence, GeneratedTraceEvent, HashEvidence,
    InvariantBindingTemplateEvidence, InvariantConfidence, InvariantPreservation,
    InvariantTemplateEvidence, InvariantTemplateSource, InvariantTier, LoopInvariantEvidence,
    ObligationEvent, ObligationEventKind, ObligationState, ObligationStatus, OpaqueLedgerEntry,
    ProofCertificate, PrunedHistoryEvidence, SelectPathEvidence, SourceEvidence,
    SourceMapAnchorEvidence, SourceMapAnchorStatus, SourceSpan, SpawnedTaskObligationEvidence,
    SuspensionStateEvidence, TemplateSchemaEvidence, TimeoutCancelEdgeEvidence, TraceEventKind,
    TranslationValidationInput, UserInvariantFactEvidence, UserInvariantPredicate,
    PROOF_CERTIFICATE_SCHEMA_VERSION, PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA,
    PROOF_TARGET_VERSION,
};

mod async_model;
mod boundaries;
mod bounded;
mod candidates;
mod core_cfg;
mod emission;
mod loop_invariants;
mod obligations;
mod source_spans;
mod templates;
mod traces;

pub use boundaries::{adapter_adjusted_replay_grade, adapter_evidence, RuntimeBoundaryEvidence};
pub use candidates::candidate_admission_evidence;
pub use emission::{emit_proof_certificate, ProofEmissionError, ProofEmissionInput};

use async_model::async_model_evidence;
use boundaries::{boundary_evidence, runtime_boundary_adapter_evidence, sort_adapter_evidence};
use bounded::{attr_name_value_fields, bounded_evidence};
use candidates::syn_path_ends_with;
use core_cfg::{
    block_index, core_cfg_edges, core_cfg_nodes, core_loop_exit_facts, core_loop_facts,
    loop_labels_by_id, parse_core_edge, LoopRegionIndex,
};
use loop_invariants::{loop_invariant_evidence, user_loop_invariant_directive};
use obligations::{
    coverage_loss, env_states, env_states_for_bindings, function_obligation_replay,
    function_summaries, obligation_evidence,
};
use source_spans::{
    line_snippet, one_based_line_for_offset, source_span_for_binding, source_span_from_kobo,
    source_span_from_range,
};
use templates::{
    invariant_confidence, invariant_template_source, lifecycle_template_version,
    template_by_binding, template_evidence, template_hash, type_by_binding,
};
use traces::{core_trace_evidence, generated_trace_evidence, trace_hashes};

pub use kobo_proof::{ArtifactKind, ReplayGrade};
