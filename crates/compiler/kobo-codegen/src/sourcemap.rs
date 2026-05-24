// Source map precision: LINE_LEVEL
// Column fields in RsSpan are line-boundary approximations (0..line_len).
// Reason: prettyplease::unparse returns a String with no token positions.
// Token-level precision requires post-format scanning; not implemented in v0.2.
// Upgrade path: replace line-level binding lookup with token scanning when it lands.

use std::path::Path;

use kobo_ir::{
    KoboSpan, OwnershipTier, ScenarioCoreTerminatorKind, ScenarioLifecycleTemplate,
    ScenarioLifecycleTemplateSource, ScenarioOpKind, ScenarioProgram,
};
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
    pub id: String,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoweringTraceEvent {
    pub id: String,
    pub function: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    pub order: u64,
    pub source_map_entry_id: String,
    pub rs_span: RsSpan,
    pub kobo_span: KoboSpan,
    pub lowering_phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
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
        lowering_trace: Vec::new(),
    }
}

pub(crate) fn build_lowering_trace(
    programs: &[ScenarioProgram],
    source_map: &KoboSourceMap,
) -> Vec<LoweringTraceEvent> {
    programs
        .iter()
        .flat_map(|program| {
            program
                .operations
                .iter()
                .enumerate()
                .filter_map(move |(order, operation)| {
                    let event = lowering_event_from_operation(&operation.kind)?;
                    let anchor = anchor_for_trace_event(
                        source_map,
                        operation.span,
                        event.binding.as_deref(),
                    )?;
                    Some(LoweringTraceEvent {
                        id: format!("lowering-{}-{order}", program.target),
                        function: program.target.clone(),
                        kind: event.kind.to_owned(),
                        binding: event.binding,
                        order: order as u64,
                        source_map_entry_id: anchor.id.clone(),
                        rs_span: anchor.rs_span.clone(),
                        kobo_span: anchor.kobo_span,
                        lowering_phase: "kobo-codegen".to_owned(),
                        template_id: event.template.map(|template| template.id.clone()),
                        template_version: event.template.map(template_version),
                    })
                })
        })
        .collect()
}

struct TraceEventSource<'a> {
    kind: &'static str,
    binding: Option<String>,
    template: Option<&'a ScenarioLifecycleTemplate>,
}

fn lowering_event_from_operation(kind: &ScenarioOpKind) -> Option<TraceEventSource<'_>> {
    match kind {
        ScenarioOpKind::CreateObligation {
            binding, template, ..
        } => Some(TraceEventSource {
            kind: "create",
            binding: Some(binding.clone()),
            template: template.as_ref(),
        }),
        ScenarioOpKind::Discharge { binding, .. } => Some(TraceEventSource {
            kind: "discharge",
            binding: Some(binding.clone()),
            template: None,
        }),
        ScenarioOpKind::Transfer { binding, .. } => Some(TraceEventSource {
            kind: "transfer",
            binding: Some(binding.clone()),
            template: None,
        }),
        ScenarioOpKind::MoveBinding { binding } => Some(TraceEventSource {
            kind: "move",
            binding: Some(binding.clone()),
            template: None,
        }),
        ScenarioOpKind::ExternalBoundary { .. } => Some(TraceEventSource {
            kind: "escape",
            binding: None,
            template: None,
        }),
        ScenarioOpKind::Return => Some(TraceEventSource {
            kind: "return",
            binding: None,
            template: None,
        }),
        ScenarioOpKind::CoreTerminator { kind, .. } => Some(TraceEventSource {
            kind: core_terminator_trace_kind(kind),
            binding: None,
            template: None,
        }),
        _ => None,
    }
}

fn core_terminator_trace_kind(kind: &ScenarioCoreTerminatorKind) -> &'static str {
    match kind {
        ScenarioCoreTerminatorKind::Return => "return",
        ScenarioCoreTerminatorKind::ErrorExit => "error_exit",
        ScenarioCoreTerminatorKind::Panic => "panic",
        ScenarioCoreTerminatorKind::Await => "cancel",
        ScenarioCoreTerminatorKind::OpaqueBoundary => "opaque_boundary",
    }
}

fn anchor_for_trace_event<'a>(
    source_map: &'a KoboSourceMap,
    span: KoboSpan,
    binding: Option<&str>,
) -> Option<&'a SourceMapEntry> {
    source_map
        .x_kobo_mappings
        .iter()
        .find(|entry| entry.kobo_span.overlaps(span) || entry.kobo_span == span)
        .or_else(|| {
            binding.and_then(|binding| {
                source_map
                    .x_kobo_mappings
                    .iter()
                    .find(|entry| entry.binding_name == binding)
            })
        })
        .or_else(|| {
            source_map
                .x_kobo_mappings
                .iter()
                .filter(|entry| entry.kobo_span.file_id == span.file_id)
                .min_by_key(|entry| entry.kobo_span.start.abs_diff(span.start))
        })
}

fn template_version(template: &ScenarioLifecycleTemplate) -> String {
    if matches!(template.source, ScenarioLifecycleTemplateSource::Inference) {
        "0.1".to_owned()
    } else {
        template.schema_version.to_string()
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

#[cfg(test)]
mod lowering_trace_tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use kobo_ir::{
        FileId, ScenarioBoundaryCallArgument, ScenarioBoundaryPolicy, ScenarioCoreTerminatorKind,
        ScenarioCoverageFacts, ScenarioExternalCallShape, ScenarioLifecycleTemplate, ScenarioOp,
        ScenarioOpKind, ScenarioProgram,
    };

    use super::{build_lowering_trace, wrap_source_map, RsSpan, SourceMapEntry};

    #[test]
    fn lowering_trace_schema_covers_translation_event_kinds() {
        let file_id = FileId(1);
        let span = kobo_ir::KoboSpan::new(10, 20, file_id);
        let source_map = wrap_source_map(
            Path::new("src/main.kobo"),
            Path::new("src/main.rs"),
            vec![SourceMapEntry {
                id: "map-0".to_owned(),
                binding_name: "delivery".to_owned(),
                kobo_span: span,
                rs_span: RsSpan {
                    line: 1,
                    column_start: 1,
                    column_end: 8,
                },
                ownership_tier: "plain".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            }],
        );
        let program = ScenarioProgram {
            file_id,
            target: "trace_case".to_owned(),
            source_hash: "source".to_owned(),
            operations: trace_kind_operations(span),
            boundaries: Vec::new(),
            coverage: ScenarioCoverageFacts::default(),
        };
        let kinds = build_lowering_trace(&[program], &source_map)
            .into_iter()
            .map(|event| event.kind)
            .collect::<BTreeSet<_>>();

        for kind in [
            "create",
            "move",
            "transfer",
            "discharge",
            "return",
            "escape",
            "panic",
            "error_exit",
            "cancel",
            "opaque_boundary",
        ] {
            assert!(
                kinds.contains(kind),
                "missing lowering trace kind {kind}: {kinds:?}"
            );
        }
    }

    fn trace_kind_operations(span: kobo_ir::KoboSpan) -> Vec<ScenarioOp> {
        let binding = "delivery".to_owned();
        vec![
            ScenarioOp {
                span,
                kind: ScenarioOpKind::CreateObligation {
                    binding: binding.clone(),
                    type_name: "Delivery".to_owned(),
                    actions: vec!["ack".to_owned()],
                    template: Some(ScenarioLifecycleTemplate::inferred(
                        "queue_delivery",
                        "queue_delivery",
                    )),
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::MoveBinding {
                    binding: binding.clone(),
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::Transfer {
                    binding: binding.clone(),
                    callee: "handoff".to_owned(),
                    proven: true,
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::Discharge {
                    binding,
                    action: "ack".to_owned(),
                },
            },
            ScenarioOp {
                span,
                kind: ScenarioOpKind::ExternalBoundary {
                    crate_name: "boundary".to_owned(),
                    call_path: Some("boundary::call".to_owned()),
                    call_arguments: Vec::<ScenarioBoundaryCallArgument>::new(),
                    return_type: None,
                    call_shape: ScenarioExternalCallShape::FreeFunction,
                    policy: ScenarioBoundaryPolicy::Opaque,
                    reason: None,
                },
            },
            terminator(span, ScenarioCoreTerminatorKind::Return),
            terminator(span, ScenarioCoreTerminatorKind::ErrorExit),
            terminator(span, ScenarioCoreTerminatorKind::Panic),
            terminator(span, ScenarioCoreTerminatorKind::Await),
            terminator(span, ScenarioCoreTerminatorKind::OpaqueBoundary),
        ]
    }

    fn terminator(span: kobo_ir::KoboSpan, kind: ScenarioCoreTerminatorKind) -> ScenarioOp {
        ScenarioOp {
            span,
            kind: ScenarioOpKind::CoreTerminator {
                kind,
                boundary: None,
                policy: None,
                edges: Vec::new(),
            },
        }
    }
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
