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
