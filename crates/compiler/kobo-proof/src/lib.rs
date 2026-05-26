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
mod rule_sync;
mod trace;
mod translation;
mod verify;

pub use bounded::{
    bounded_wording, classify_bounded_completeness, normalized_bound_hash,
    BoundedClassificationInput,
};
pub use certificate::{
    AdapterConfidence, AdapterEvidence, ArtifactKind, AsyncModelEvidence, BoundDeclaration,
    BoundDimension, BoundSource, BoundaryAssumption, BoundaryPolicy, BoundedCompleteness,
    BoundedHistoryEvidence, BoundedProofEvidence, CancelEdgeEvidence, CandidateAdmissionEvidence,
    CandidateAdmissionFact, CoreCfgEdge, CoreCfgNode, CoreEvidence, CoreLoopBackEdgeFact,
    CoreLoopExitFact, CoreTraceEvent, CoverageLoss, FunctionSummary, FutureStateLocalEvidence,
    FutureStateObligationEvidence, GeneratedTraceEvent, HashEvidence,
    InvariantBindingTemplateEvidence, InvariantConfidence, InvariantPreservation,
    InvariantTemplateEvidence, InvariantTemplateSource, InvariantTier, LoopInvariantEvidence,
    ObligationEvent, ObligationEventKind, ObligationState, ObligationStatus, OpaqueLedgerEntry,
    ProofCertificate, PrunedHistoryEvidence, ReplayGrade, SelectPathEvidence, SourceEvidence,
    SourceMapAnchorEvidence, SourceMapAnchorStatus, SourceSpan, SpawnedTaskObligationEvidence,
    SuspensionStateEvidence, TemplateSchemaEvidence, TimeoutCancelEdgeEvidence, TraceEventKind,
    TraceMismatchEvidence, TraceMismatchKind, TranslationValidationEvidence,
    TranslationValidationStatus, UserInvariantFactEvidence, UserInvariantPredicate,
    PROOF_CERTIFICATE_SCHEMA_VERSION, PROOF_CLAIM_SCOPE, PROOF_SEMANTIC_SCHEMA,
    PROOF_TARGET_VERSION,
};
pub use error::VerificationError;
pub use hash::{
    certificate_material_hash, core_material_hash, stable_hash, template_schema_hash,
    trace_material_hash,
};
pub use rule_sync::{
    load_lean_rule_manifest, load_obligation_rule_catalog, parse_lean_rule_manifest_sources,
    parse_obligation_rule_catalog, required_obligation_rule_ids, validate_obligation_rule_catalog,
    validate_obligation_rule_catalog_against_lean_manifest, LeanRuleManifest, LeanRuleShape,
    ObligationRule, ObligationRuleCatalog, RuleSyncError,
};
pub use translation::{derive_translation_validation, TranslationValidationInput};
pub use verify::{
    parse_certificate_json, verify_certificate, verify_certificate_header, VerificationContext,
    VerificationReport,
};
