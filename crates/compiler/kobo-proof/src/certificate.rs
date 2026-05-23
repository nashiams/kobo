use serde::{Deserialize, Serialize};

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
    pub template_versions: Vec<TemplateVersionEvidence>,
    pub boundary_assumption_hashes: Vec<HashEvidence>,
    pub boundary_assumptions: Vec<BoundaryAssumption>,
    pub adapter_confidence: Vec<AdapterEvidence>,
    pub obligation_events: Vec<ObligationEvent>,
    pub entry_env: Vec<ObligationState>,
    pub exit_env: Vec<ObligationState>,
    pub function_summaries: Vec<FunctionSummary>,
    pub coverage_loss: Vec<CoverageLoss>,
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
    pub state: String,
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
    pub path_kind: String,
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
pub struct TemplateVersionEvidence {
    pub id: String,
    pub kind: String,
    pub version: String,
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
    pub state: String,
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
