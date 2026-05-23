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
    pub adapter: String,
    pub confidence: AdapterConfidence,
    pub replay_grade: ReplayGrade,
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
