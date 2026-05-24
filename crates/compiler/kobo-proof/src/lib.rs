mod adapter;
mod async_model;
mod boundary;
mod bounded;
mod candidate;
mod certificate;
mod error;
mod hash;
mod invariant;
mod obligation;
mod trace;
mod translation;
mod verify;

pub use bounded::normalized_bound_hash;
pub use certificate::{
    AdapterConfidence, AdapterEvidence, ArtifactKind, AsyncModelEvidence, BoundDeclaration,
    BoundDimension, BoundSource, BoundaryAssumption, BoundaryPolicy, BoundedCompleteness,
    BoundedHistoryEvidence, BoundedProofEvidence, CancelEdgeEvidence, CandidateAdmissionEvidence,
    CandidateAdmissionFact, CoreCfgEdge, CoreCfgNode, CoreEvidence, CoreLoopBackEdgeFact,
    CoreLoopExitFact, CoreTraceEvent, CoverageLoss, FunctionSummary, FutureStateLocalEvidence,
    FutureStateObligationEvidence, GeneratedTraceEvent, HashEvidence, InvariantConfidence,
    InvariantPreservation, InvariantTemplateEvidence, InvariantTemplateSource, InvariantTier,
    LoopInvariantEvidence, ObligationEvent, ObligationEventKind, ObligationState, ObligationStatus,
    OpaqueLedgerEntry, ProofCertificate, PrunedHistoryEvidence, ReplayGrade, SelectPathEvidence,
    SourceEvidence, SourceMapAnchorEvidence, SourceMapAnchorStatus, SourceSpan,
    SpawnedTaskObligationEvidence, SuspensionStateEvidence, TemplateSchemaEvidence,
    TimeoutCancelEdgeEvidence, TraceEventKind, TraceMismatchEvidence, TraceMismatchKind,
    TranslationValidationEvidence, TranslationValidationStatus, PROOF_CERTIFICATE_SCHEMA_VERSION,
    PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA, PROOF_TARGET_VERSION,
};
pub use error::VerificationError;
pub use hash::{
    certificate_material_hash, core_material_hash, stable_hash, template_schema_hash,
    trace_material_hash,
};
pub use verify::{
    parse_certificate_json, verify_certificate, verify_certificate_header, VerificationContext,
    VerificationReport,
};
