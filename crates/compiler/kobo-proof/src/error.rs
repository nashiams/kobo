#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VerificationError {
    #[error("source hash is missing")]
    MissingSourceHash,
    #[error("source hash mismatch: expected {expected}, observed {observed}")]
    SourceHashMismatch { expected: String, observed: String },
    #[error("core hash mismatch: expected {expected}, observed {observed}")]
    CoreHashMismatch { expected: String, observed: String },
    #[error("unsupported certificate header {field}: expected {expected}, observed {observed}")]
    UnsupportedCertificateHeader {
        field: String,
        expected: String,
        observed: String,
    },
    #[error("unsupported certificate field {field}: expected {expected}, observed {observed}")]
    UnsupportedCertificateField {
        field: String,
        expected: String,
        observed: String,
    },
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
    #[error("template {id} has unsupported schema {template_schema}@{schema_version}")]
    UnsupportedTemplateSchema {
        id: String,
        template_schema: String,
        schema_version: u64,
    },
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
    #[error("CFG edge transition mismatch for {edge}: {reason}")]
    CfgEdgeTransitionMismatch { edge: String, reason: String },
    #[error("loop {loop_id} is missing a proof-grade back-edge fact")]
    MissingLoopBackEdgeFact { loop_id: String },
    #[error("loop {loop_id} inferred invariant is missing template evidence")]
    MissingInvariantTemplate { loop_id: String },
    #[error("loop {loop_id} invariant was not preserved: {reason}")]
    InvariantNotPreserved { loop_id: String, reason: String },
    #[error("loop {loop_id} leaks unresolved obligation {binding} at the back-edge")]
    LoopBackEdgeLeak { loop_id: String, binding: String },
    #[error("bounded evidence {evidence_id} cannot claim proof with {completeness} enumeration: {wording}")]
    IncompleteBoundedEnumeration {
        evidence_id: String,
        completeness: String,
        wording: String,
    },
    #[error("bounded evidence {evidence_id} is missing proof-relevant bounds")]
    MissingBoundedProofDimension { evidence_id: String },
    #[error("bounded evidence {evidence_id} has inconsistent history count: expected {expected}, observed {observed}")]
    BoundedHistoryCountMismatch {
        evidence_id: String,
        expected: u64,
        observed: u64,
    },
    #[error("bounded evidence {evidence_id} normalized bound hash mismatch: expected {expected}, observed {observed}")]
    BoundedEvidenceHashMismatch {
        evidence_id: String,
        expected: String,
        observed: String,
    },
    #[error("translation validation is missing generated event for Core event {core_event_id}")]
    TranslationTraceMissingEvent { core_event_id: String },
    #[error("translation validation has extra generated event {generated_event_id}")]
    TranslationTraceExtraEvent { generated_event_id: String },
    #[error("translation validation order mismatch for Core event {core_event_id}: expected {expected}, observed {observed}")]
    TranslationTraceOrderMismatch {
        core_event_id: String,
        expected: u64,
        observed: u64,
    },
    #[error("translation validation kind mismatch for Core event {core_event_id}: expected {expected}, observed {observed}")]
    TranslationTraceKindMismatch {
        core_event_id: String,
        expected: String,
        observed: String,
    },
    #[error("translation validation binding mismatch for Core event {core_event_id}: expected {expected}, observed {observed}")]
    TranslationTraceBindingMismatch {
        core_event_id: String,
        expected: String,
        observed: String,
    },
    #[error("translation validation template mismatch for Core event {core_event_id}: expected {expected}, observed {observed}")]
    TranslationTraceTemplateMismatch {
        core_event_id: String,
        expected: String,
        observed: String,
    },
    #[error("translation validation source-map anchor {anchor_id} for generated event {generated_event_id} is {status}")]
    TranslationSourceMapAnchorMismatch {
        generated_event_id: String,
        anchor_id: String,
        status: String,
    },
    #[error("translation validation status mismatch: expected {expected}, observed {observed}")]
    TranslationValidationStatusMismatch { expected: String, observed: String },
    #[error("translation validation trace hash mismatch for {trace_id}: expected {expected}, observed {observed}")]
    TranslationTraceHashMismatch {
        trace_id: String,
        expected: String,
        observed: String,
    },
    #[error("removed discharge for {binding}")]
    RemovedDischarge { binding: String },
    #[error("unresolved obligation {binding} reaches a modeled exit as {state}")]
    UnresolvedExitObligation { binding: String, state: String },
    #[error("unknown obligation event kind: {message}")]
    UnknownEventKind { message: String },
    #[error("unknown boundary policy: {message}")]
    UnknownBoundaryPolicy { message: String },
    #[error("unknown unversioned certificate field: {message}")]
    UnknownField { message: String },
    #[error("certificate JSON parse error: {message}")]
    Parse { message: String },
}
