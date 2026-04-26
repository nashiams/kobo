use crate::lower::{AnnotationNote, LoweringSite, ResolvedAnchorMap};
use crate::sourcemap::SourceMapEntry;

pub(crate) fn annotate(
    formatted: &str,
    sites: &[LoweringSite],
    notes: &[AnnotationNote],
    anchors: &ResolvedAnchorMap,
    entries: &mut [SourceMapEntry],
) -> String {
    debug_assert_eq!(sites.len(), entries.len());

    let had_trailing_newline = formatted.ends_with('\n');
    let mut lines: Vec<String> = formatted.lines().map(str::to_owned).collect();
    let mut inserted_before = 0usize;

    for (site, entry) in sites.iter().zip(entries.iter_mut()) {
        let insertion_index = entry
            .rs_span
            .line
            .saturating_sub(1)
            .saturating_add(inserted_before)
            .min(lines.len());
        lines.insert(
            insertion_index,
            format!(
                "// kobo: {} @ line {} -> {} ({})",
                site.display_name(),
                site.kobo_line,
                entry.ownership_tier,
                site.reason
            ),
        );
        entry.rs_span.line += inserted_before + 1;
        inserted_before += 1;
    }

    let site_anchor_lines = sites
        .iter()
        .filter_map(|site| anchors.get(site.node).map(|anchor| anchor.line))
        .collect::<Vec<_>>();
    let mut ordered_notes = notes.iter().enumerate().collect::<Vec<_>>();
    ordered_notes.sort_by_key(|(index, note)| {
        let line = anchors
            .get(note.node)
            .map(|anchor| anchor.line)
            .unwrap_or(usize::MAX);
        (line, *index)
    });
    let mut inserted_notes = 0usize;

    for (_, note) in ordered_notes {
        let anchor = anchors.get(note.node).unwrap_or_else(|| {
            unreachable!("invariant: every annotation note must resolve to an anchor")
        });
        let prior_site_insertions = site_anchor_lines
            .iter()
            .filter(|line| **line < anchor.line)
            .count();
        let insertion_index = anchor
            .line
            .saturating_sub(1)
            .saturating_add(prior_site_insertions)
            .saturating_add(inserted_notes)
            .min(lines.len());
        let inserted_line = insertion_index + 1;
        lines.insert(
            insertion_index,
            format!(
                "// kobo: {} @ line {} -> {}",
                note.display_name(),
                note.kobo_line,
                note.reason
            ),
        );
        for entry in entries.iter_mut() {
            if entry.rs_span.line >= inserted_line {
                entry.rs_span.line += 1;
            }
        }
        inserted_notes += 1;
    }

    let mut annotated = lines.join("\n");
    if had_trailing_newline || !annotated.is_empty() {
        annotated.push('\n');
    }

    annotated
}

#[cfg(test)]
mod tests {
    use kobo_ir::{FileId, KoboSpan, OwnershipTier};

    use crate::lower::{AnnotationNote, LoweringSite, ResolvedAnchor, ResolvedAnchorMap};
    use crate::sourcemap::{RsSpan, SourceMapEntry};

    use super::annotate;

    #[test]
    fn notes_shift_existing_source_map_entries() {
        let formatted = "fn main() {\n    let x = value();\n    let y = x.clone();\n}\n";
        let mut anchors = ResolvedAnchorMap::default();
        anchors.insert(
            kobo_ir::KirNodeId(1),
            ResolvedAnchor {
                line: 2,
                column_start: 4,
                column_end: 5,
            },
        );
        anchors.insert(
            kobo_ir::KirNodeId(2),
            ResolvedAnchor {
                line: 3,
                column_start: 4,
                column_end: 5,
            },
        );
        let sites = vec![
            LoweringSite::new(
                kobo_ir::KirNodeId(1),
                "x",
                OwnershipTier::RcShared,
                KoboSpan::new(0, 1, FileId(0)),
                1,
                "read-only shared across 2 call sites",
            ),
            LoweringSite::new(
                kobo_ir::KirNodeId(2),
                "y",
                OwnershipTier::PlainOwned,
                KoboSpan::new(2, 3, FileId(0)),
                2,
                "local-only non-Copy binding",
            ),
        ];
        let notes = vec![AnnotationNote::new(
            kobo_ir::KirNodeId(1),
            "x",
            1,
            "clone-elision fallback: move safety check failed - conservative clone",
        )];
        let mut entries = vec![
            SourceMapEntry {
                rs_span: RsSpan {
                    line: 2,
                    column_start: 0,
                    column_end: 19,
                },
                kobo_span: KoboSpan::new(0, 1, FileId(0)),
                ownership_tier: "rc".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            },
            SourceMapEntry {
                rs_span: RsSpan {
                    line: 3,
                    column_start: 0,
                    column_end: 19,
                },
                kobo_span: KoboSpan::new(2, 3, FileId(0)),
                ownership_tier: "plain".to_owned(),
                solver_outcome: None,
                decision_source: None,
                solver_node_id: None,
            },
        ];

        let annotated = annotate(formatted, &sites, &notes, &anchors, &mut entries);

        assert!(annotated.contains("clone-elision fallback"));
        assert_eq!(entries[0].rs_span.line, 4);
        assert_eq!(entries[1].rs_span.line, 6);
    }
}
