#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VerificationError {
    #[error("source hash is missing")]
    MissingSourceHash,
    #[error("source hash mismatch: expected {expected}, observed {observed}")]
    SourceHashMismatch { expected: String, observed: String },
    #[error("core hash mismatch: expected {expected}, observed {observed}")]
    CoreHashMismatch { expected: String, observed: String },
    #[error("missing cancel edge for suspension block {block}")]
    MissingCancelEdge { block: String },
    #[error("missing async cancel evidence for suspension block {block}")]
    MissingAsyncCancelEvidence { block: String },
    #[error("async evidence mismatch in {field} for {id}")]
    AsyncEvidenceMismatch { field: String, id: String },
    #[error("missing future state obligation {binding} for suspension state {suspension_state}")]
    MissingFutureStateObligation {
        binding: String,
        suspension_state: String,
    },
    #[error("missing select path evidence for block {block} path {path_kind}")]
    MissingSelectPathEvidence { block: String, path_kind: String },
    #[error("template {id} has stale version {version}")]
    StaleTemplateVersion { id: String, version: String },
    #[error("template hash mismatch for {id}: expected {expected}, observed {observed}")]
    TemplateHashMismatch {
        id: String,
        expected: String,
        observed: String,
    },
    #[error("boundary hash mismatch for {id}: expected {expected}, observed {observed}")]
    BoundaryHashMismatch {
        id: String,
        expected: String,
        observed: String,
    },
    #[error("opaque boundary {boundary} lacks a ledger entry")]
    OpaqueEdgeWithoutLedger { boundary: String },
    #[error("exact replay crosses disallowed boundary {boundary} with policy {policy}")]
    ExactReplayWithDebtBoundary { boundary: String, policy: String },
    #[error("exact replay has coverage loss {kind}:{label}")]
    ExactReplayWithCoverageLoss { kind: String, label: String },
    #[error("exact replay crosses adapter {adapter} with confidence {confidence}")]
    ExactReplayWithAdapterConfidence { adapter: String, confidence: String },
    #[error("exact replay crosses unsupported adapter boundary {boundary}")]
    ExactReplayMissingAdapterEvidence { boundary: String },
    #[error("metadata-only adapter {adapter} cannot claim replayable behavior")]
    MetadataOnlyAdapterReplayable { adapter: String },
    #[error("sampled adapter {adapter} must emit probing_pass evidence")]
    SampledAdapterWithoutProbingPass { adapter: String },
    #[error("stale adapter {adapter} requires debt or not_replayable replay grade")]
    StaleAdapterReplayable { adapter: String },
    #[error("candidate {id} missing admission evidence: {gate}")]
    CandidateAdmissionMissing { id: String, gate: String },
    #[error("certificate material hash mismatch: expected {expected}, observed {observed}")]
    CertificateMaterialHashMismatch { expected: String, observed: String },
    #[error("obligation replay mismatch for {binding}: expected {expected}, observed {observed}")]
    ObligationReplayMismatch {
        binding: String,
        expected: String,
        observed: String,
    },
    #[error("removed discharge for {binding}")]
    RemovedDischarge { binding: String },
    #[error("unresolved obligation {binding} reaches a modeled exit")]
    UnresolvedExitObligation { binding: String },
    #[error("unknown obligation event kind: {message}")]
    UnknownEventKind { message: String },
    #[error("unknown boundary policy: {message}")]
    UnknownBoundaryPolicy { message: String },
    #[error("unknown unversioned certificate field: {message}")]
    UnknownField { message: String },
    #[error("certificate JSON parse error: {message}")]
    Parse { message: String },
}
