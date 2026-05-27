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

use async_model::*;
use boundaries::*;
use bounded::*;
use candidates::*;
use core_cfg::*;
use loop_invariants::*;
use obligations::*;
use source_spans::*;
use templates::*;
use traces::*;

pub use kobo_proof::{ArtifactKind, ReplayGrade};
