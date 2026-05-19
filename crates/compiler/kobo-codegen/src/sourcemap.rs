// Source map precision: LINE_LEVEL
// Column fields in RsSpan are line-boundary approximations (0..line_len).
// Reason: prettyplease::unparse returns a String with no token positions.
// Token-level precision requires post-format scanning; not implemented in v0.2.
// Upgrade path: replace line-level binding lookup with token scanning when it lands.

use std::path::Path;

use kobo_ir::{KoboSpan, OwnershipTier};
use serde::{Deserialize, Serialize};

use crate::lower::{LoweringSite, ResolvedAnchorMap};
use crate::RuntimeEvidence;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RsSpan {
    pub line: usize,
    pub column_start: usize,
    pub column_end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceMapEntry {
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

pub fn wrap_source_map(
    kobo_path: &Path,
    rs_path: &Path,
    entries: Vec<SourceMapEntry>,
) -> KoboSourceMap {
    KoboSourceMap {
        version: 3,
        file: rs_path.display().to_string(),
        sources: vec![kobo_path.display().to_string()],
        x_kobo_mappings: entries,
        runtime_evidence: None,
        solver_evidence: None,
    }
}

impl KoboSourceMap {
    pub fn generated_file(&self) -> &str {
        &self.file
    }

    pub fn kobo_path(&self) -> &str {
        self.sources
            .first()
            .map(String::as_str)
            .unwrap_or(self.file.as_str())
    }

    pub fn lookup_kobo_span(&self, rs_span: RsSpan) -> Option<KoboSpan> {
        self.x_kobo_mappings
            .iter()
            .find(|entry| {
                entry.rs_span.line == rs_span.line && spans_overlap(&entry.rs_span, &rs_span)
            })
            .map(|entry| entry.kobo_span)
            .or_else(|| {
                self.x_kobo_mappings
                    .iter()
                    .find(|entry| entry.rs_span.line == rs_span.line)
                    .map(|entry| entry.kobo_span)
            })
    }

    pub fn lookup_rs_spans(&self, kobo_span: KoboSpan) -> Vec<RsSpan> {
        self.x_kobo_mappings
            .iter()
            .filter(|entry| entry.kobo_span.overlaps(kobo_span) || entry.kobo_span == kobo_span)
            .map(|entry| entry.rs_span.clone())
            .collect()
    }

    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

fn spans_overlap(left: &RsSpan, right: &RsSpan) -> bool {
    left.column_start <= right.column_end && right.column_start <= left.column_end
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

#[cfg(test)]
mod tests {
    use kobo_ir::{KoboSpan, OwnershipTier};

    use crate::lower::{LoweringSite, ResolvedAnchor, ResolvedAnchorMap};

    use super::{build_source_map_entries, wrap_source_map, RsSpan};

    #[test]
    fn source_map_supports_forward_and_reverse_lookup() {
        let mut anchors = ResolvedAnchorMap::default();
        anchors.insert(
            kobo_ir::KirNodeId(1),
            ResolvedAnchor {
                line: 2,
                column_start: 4,
                column_end: 9,
            },
        );
        let entries = build_source_map_entries(
            &[LoweringSite::new(
                kobo_ir::KirNodeId(1),
                "names",
                OwnershipTier::RcMutShared,
                KoboSpan::new(4, 9, kobo_ir::FileId(0)),
                2,
                "non-Copy shared binding",
            )],
            &anchors,
        );
        let source_map = wrap_source_map("src/main.kobo".as_ref(), "src/main.rs".as_ref(), entries);
        assert_eq!(source_map.generated_file(), "src/main.rs");
        assert_eq!(source_map.kobo_path(), "src/main.kobo");

        let kobo_span = source_map
            .lookup_kobo_span(RsSpan {
                line: 2,
                column_start: 0,
                column_end: 47,
            })
            .expect("mapped line should resolve");
        assert_eq!(kobo_span, KoboSpan::new(4, 9, kobo_ir::FileId(0)));

        let rs_spans = source_map.lookup_rs_spans(KoboSpan::new(4, 9, kobo_ir::FileId(0)));
        assert_eq!(rs_spans.len(), 1);
        assert_eq!(rs_spans[0].line, 2);
    }
}
