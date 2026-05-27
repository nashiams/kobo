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
