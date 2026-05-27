use kobo_ir::{KoboSpan, OwnershipTier};
use serde::{Deserialize, Serialize};

use crate::lower::{LoweringSite, ResolvedAnchorMap};
use crate::RuntimeEvidence;

use super::lowering_trace::LoweringTraceEvent;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RsSpan {
    pub line: usize,
    pub column_start: usize,
    pub column_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMapEntry {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core_event_id: Option<String>,
    pub binding_name: String,
    pub rs_span: RsSpan,
    pub kobo_span: KoboSpan,
    pub ownership_tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_node_id: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolverEvidenceJson {
    pub outcome: String,
    pub graph_fingerprint: String,
    pub node_count: u64,
    pub edge_count: u64,
    pub budget: SolverBudgetJson,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SolverBudgetJson {
    pub max_cluster_size: u64,
    pub budget_seconds: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KoboSourceMap {
    pub version: u32,
    pub file: String,
    pub sources: Vec<String>,
    #[serde(rename = "x_kobo_mappings")]
    pub x_kobo_mappings: Vec<SourceMapEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_evidence: Option<RuntimeEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solver_evidence: Option<SolverEvidenceJson>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lowering_trace: Vec<LoweringTraceEvent>,
}

pub(crate) fn build_source_map_entries(
    sites: &[LoweringSite],
    anchors: &ResolvedAnchorMap,
) -> Vec<SourceMapEntry> {
    let mut entries = Vec::with_capacity(sites.len());

    for site in sites {
        let anchor = anchors.get(site.node).unwrap_or_else(|| {
            panic!(
                "invariant: every lowering site must resolve to an anchor; missing node {:?} binding `{}` span {:?}",
                site.node, site.binding_name, site.kobo_span
            )
        });

        entries.push(SourceMapEntry {
            id: format!("map-{}", site.node.0),
            core_event_id: None,
            binding_name: site.binding_name.clone(),
            rs_span: RsSpan {
                line: anchor.line,
                column_start: anchor.column_start,
                column_end: anchor.column_end,
            },
            kobo_span: site.kobo_span,
            ownership_tier: ownership_tier_label(site.ownership_tier).to_owned(),
            solver_outcome: None,
            decision_source: None,
            solver_node_id: Some(site.node.0 as u64),
        });
    }

    entries
}

fn ownership_tier_label(tier: OwnershipTier) -> &'static str {
    match tier {
        OwnershipTier::PlainOwned => "plain",
        OwnershipTier::BoxOwned => "box",
        OwnershipTier::RcShared => "rc",
        OwnershipTier::ArcShared => "arc",
        OwnershipTier::RcMutShared => "rc_refcell",
        OwnershipTier::ArcMutShared => "arc_rwlock",
        OwnershipTier::Scoped => "scoped_handle",
        OwnershipTier::Undecided => "plain",
    }
}
