use serde::{Deserialize, Serialize};

pub const PROOF_CERTIFICATE_SCHEMA_VERSION: u32 = 2;
pub const PROOF_TARGET_VERSION: &str = "kobo-core-obligation-flow-1";
pub const PROOF_SEMANTIC_SCHEMA: &str = ".kproof";
pub const PROOF_CLAIM_SCOPE: &str = "modeled_core_obligation_flow_only";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofCertificate {
    pub schema_version: u32,
    pub proof_target_version: String,
    pub semantic_schema: String,
    pub artifact_kind: ArtifactKind,
    pub claim_scope: String,
    pub compiler_version: String,
    pub source: SourceEvidence,
    pub core: CoreEvidence,
    pub replay_grade: ReplayGrade,
    pub template_hashes: Vec<HashEvidence>,
    pub template_schemas: Vec<TemplateSchemaEvidence>,
    pub boundary_assumption_hashes: Vec<HashEvidence>,
    pub boundary_assumptions: Vec<BoundaryAssumption>,
    pub adapter_confidence: Vec<AdapterEvidence>,
    pub obligation_events: Vec<ObligationEvent>,
    pub entry_env: Vec<ObligationState>,
    pub exit_env: Vec<ObligationState>,
    pub function_summaries: Vec<FunctionSummary>,
    pub coverage_loss: Vec<CoverageLoss>,
    #[serde(default)]
    pub loop_invariants: Vec<LoopInvariantEvidence>,
    #[serde(default)]
    pub bounded_evidence: Vec<BoundedProofEvidence>,
    #[serde(default)]
    pub core_obligation_trace: Vec<CoreTraceEvent>,
    #[serde(default)]
    pub generated_rust_trace: Vec<GeneratedTraceEvent>,
    #[serde(default)]
    pub translation_validation: TranslationValidationEvidence,
    pub opaque_edge_ledger: Vec<OpaqueLedgerEntry>,
    pub candidate_admission: Vec<CandidateAdmissionEvidence>,
    pub certificate_material_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEvidence {
    pub path: String,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreEvidence {
    pub hash: String,
    pub version: String,
    pub cfg_nodes: Vec<CoreCfgNode>,
    pub cfg_edges: Vec<CoreCfgEdge>,
    #[serde(default)]
    pub loop_facts: Vec<CoreLoopBackEdgeFact>,
    pub async_model: AsyncModelEvidence,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsyncModelEvidence {
    pub suspension_states: Vec<SuspensionStateEvidence>,
    pub future_state_locals: Vec<FutureStateLocalEvidence>,
    pub future_state_obligations: Vec<FutureStateObligationEvidence>,
    pub cancel_edges: Vec<CancelEdgeEvidence>,
    pub select_paths: Vec<SelectPathEvidence>,
    pub timeout_cancel_edges: Vec<TimeoutCancelEdgeEvidence>,
    pub spawned_task_obligations: Vec<SpawnedTaskObligationEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuspensionStateEvidence {
    pub id: String,
    pub function: String,
    pub block: String,
    pub terminator_kind: String,
    pub boundary: Option<String>,
    pub resume_edge: String,
    pub cancel_edge: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FutureStateLocalEvidence {
    pub binding: String,
    pub suspension_state: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FutureStateObligationEvidence {
    pub binding: String,
    pub state: ObligationStatus,
    pub suspension_state: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelEdgeEvidence {
    pub id: String,
    pub function: String,
    pub from: String,
    pub to: String,
    pub reason: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectPathEvidence {
    pub id: String,
    pub function: String,
    pub branch_block: String,
    pub branch_target: String,
    pub path_kind: String,
    pub obligation_results: Vec<ObligationState>,
    pub cancelled_obligations: Vec<ObligationState>,
    pub obligation_result_hash: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutCancelEdgeEvidence {
    pub id: String,
    pub function: String,
    pub suspension_state: String,
    pub source: String,
    pub cancel_edge: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnedTaskObligationEvidence {
    pub binding: String,
    pub required_resolution: Vec<String>,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAdmissionEvidence {
    pub id: String,
    pub track: String,
    pub status: String,
    pub evidence: Vec<CandidateAdmissionFact>,
    pub inspect_visibility: Option<String>,
    pub manual_rust_equivalent: Option<String>,
    pub strict_compatible: bool,
    pub whole_ecosystem_modeling_required: bool,
    pub diagnostic_snapshots: Vec<String>,
    pub replay_related: bool,
    pub replay_grade: Option<ReplayGrade>,
    pub adapter_confidence: Vec<AdapterEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAdmissionFact {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreCfgNode {
    pub id: String,
    pub function: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreCfgEdge {
    pub id: String,
    pub function: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreLoopBackEdgeFact {
    pub id: String,
    pub function: String,
    pub entry_block: String,
    pub back_edge_source: String,
    pub back_edge_target: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub path: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub mapped: bool,
    pub snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashEvidence {
    pub id: String,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSchemaEvidence {
    pub id: String,
    pub kind: String,
    pub template_schema: String,
    pub schema_version: u64,
    pub confidence: String,
    pub source: String,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryAssumption {
    pub id: String,
    pub boundary: String,
    pub policy: BoundaryPolicy,
    pub reason: Option<String>,
    pub source_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterEvidence {
    pub boundary: String,
    pub adapter: String,
    pub version: Option<String>,
    pub confidence: AdapterConfidence,
    pub replay_grade: ReplayGrade,
    pub outcome: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationEvent {
    pub id: String,
    pub kind: ObligationEventKind,
    pub binding: Option<String>,
    pub action: Option<String>,
    pub source_span: SourceSpan,
    pub state_before: Vec<ObligationState>,
    pub state_after: Vec<ObligationState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationState {
    pub binding: String,
    pub state: ObligationStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionSummary {
    pub function: String,
    pub event_count: usize,
    pub entry_env: Vec<ObligationState>,
    pub exit_env: Vec<ObligationState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageLoss {
    pub kind: String,
    pub label: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopInvariantEvidence {
    pub id: String,
    pub function: String,
    pub entry_block: String,
    pub back_edge_source: String,
    pub back_edge_target: String,
    pub tier: InvariantTier,
    pub expression: String,
    pub source_span: SourceSpan,
    pub obligations_created: Vec<String>,
    pub back_edge_states: Vec<ObligationState>,
    pub preservation: InvariantPreservation,
    pub template: Option<InvariantTemplateEvidence>,
    pub downgrade_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvariantTemplateEvidence {
    pub id: String,
    pub version: String,
    pub schema_hash: String,
    pub source: InvariantTemplateSource,
    pub confidence: InvariantConfidence,
    pub obligation_kind: String,
    pub lifecycle_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundedProofEvidence {
    pub id: String,
    pub function: String,
    pub bounds: Vec<BoundDeclaration>,
    pub normalized_bound_hash: String,
    pub enumerated_history_count: u64,
    pub expected_complete_history_count: Option<u64>,
    pub scheduler_dimensions: Vec<String>,
    pub fault_dimensions: Vec<String>,
    pub cancellation_points: Vec<String>,
    pub pruned_histories: Vec<PrunedHistoryEvidence>,
    pub completeness: BoundedCompleteness,
    pub wording: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundDeclaration {
    pub dimension: BoundDimension,
    pub value: u64,
    pub source: BoundSource,
    pub proof_relevant: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrunedHistoryEvidence {
    pub id: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreTraceEvent {
    pub id: String,
    pub kind: TraceEventKind,
    pub binding: Option<String>,
    pub order: u64,
    pub source_span: SourceSpan,
    pub template_id: Option<String>,
    pub template_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedTraceEvent {
    pub id: String,
    pub core_event_id: String,
    pub kind: TraceEventKind,
    pub binding: Option<String>,
    pub order: u64,
    pub source_map_anchor: SourceMapAnchorEvidence,
    pub lowering_phase: String,
    pub template_id: Option<String>,
    pub template_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMapAnchorEvidence {
    pub id: String,
    pub status: SourceMapAnchorStatus,
    pub generated_span: SourceSpan,
    pub kobo_span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranslationValidationEvidence {
    pub status: TranslationValidationStatus,
    pub mismatches: Vec<TraceMismatchEvidence>,
}

impl Default for TranslationValidationEvidence {
    fn default() -> Self {
        Self {
            status: TranslationValidationStatus::CoreOnly,
            mismatches: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceMismatchEvidence {
    pub kind: TraceMismatchKind,
    pub core_event_id: Option<String>,
    pub generated_event_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueLedgerEntry {
    pub edge_id: String,
    pub boundary: String,
    pub evidence_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    #[serde(rename = "kproof")]
    Kproof,
    #[serde(rename = "kwit.proof.json")]
    KwitProofJson,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayGrade {
    Exact,
    Partial,
    NotReplayable,
    Debt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdapterConfidence {
    Exact,
    Modeled,
    Sampled,
    MetadataOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    Typed,
    Model,
    Record,
    Activity,
    Stub,
    Outside,
    Opaque,
    Debt,
    Unselected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantTier {
    Inferred,
    User,
    Bounded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantPreservation {
    Preserved,
    Failed,
    Downgraded,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantTemplateSource {
    BuiltIn,
    Declaration,
    Adapter,
    Summary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantConfidence {
    Exact,
    Modeled,
    Sampled,
    MetadataOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundDimension {
    LoopIterations,
    QueueCapacity,
    MessageCount,
    SchedulerHistories,
    CancellationPoints,
    RetryAttempts,
    TimeoutPaths,
    FaultInjectionChoices,
    ExternalBoundaryRecordings,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundSource {
    Ward,
    Template,
    Adapter,
    Summary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundedCompleteness {
    Complete,
    Incomplete,
    Sampled,
    Timeout,
}

impl BoundedCompleteness {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Sampled => "sampled",
            Self::Timeout => "timeout",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceEventKind {
    Create,
    Move,
    Transfer,
    Discharge,
    Return,
    Escape,
    Panic,
    ErrorExit,
    Cancel,
    OpaqueBoundary,
}

impl TraceEventKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Move => "move",
            Self::Transfer => "transfer",
            Self::Discharge => "discharge",
            Self::Return => "return",
            Self::Escape => "escape",
            Self::Panic => "panic",
            Self::ErrorExit => "error_exit",
            Self::Cancel => "cancel",
            Self::OpaqueBoundary => "opaque_boundary",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMapAnchorStatus {
    Mapped,
    Missing,
    Stale,
}

impl SourceMapAnchorStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Mapped => "mapped",
            Self::Missing => "missing",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationValidationStatus {
    CoreOnly,
    Validated,
    Failed,
    NotGenerated,
}

impl TranslationValidationStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::CoreOnly => "core_only",
            Self::Validated => "validated",
            Self::Failed => "failed",
            Self::NotGenerated => "not_generated",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceMismatchKind {
    MissingEvent,
    ExtraEvent,
    OrderMismatch,
    KindMismatch,
    BindingMismatch,
    SourceMapAnchorMismatch,
    TemplateMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationEventKind {
    Create,
    Discharge,
    Transfer,
    Move,
    BranchUnresolved,
    Escape,
    UnsupportedContainer,
    Call,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStatus {
    Owned,
    Resolved,
    Transferred,
    Moved,
    BranchUnresolved,
    Escaped,
}

impl ObligationStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Resolved => "resolved",
            Self::Transferred => "transferred",
            Self::Moved => "moved",
            Self::BranchUnresolved => "branch_unresolved",
            Self::Escaped => "escaped",
        }
    }

    pub const fn is_unresolved_exit(&self) -> bool {
        matches!(self, Self::Owned | Self::Moved | Self::BranchUnresolved)
    }
}
