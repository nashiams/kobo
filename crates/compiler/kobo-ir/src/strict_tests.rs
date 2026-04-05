/// P2 IR type tests — written FAILING first (TDD).
///
/// Run with: cargo test -p kobo-ir
#[cfg(test)]
mod tests {
    use crate::node_id::{FileId, KirNodeId};
    use crate::span::KoboSpan;
    use crate::strict::{
        CaptureAccessKind, CaptureSet, CapturedBinding, StrictBoundaryFact,
        StrictBoundaryViolation,
    };

    fn span(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(0))
    }

    fn node(id: u32) -> KirNodeId {
        KirNodeId(id)
    }

    // Test 1: CaptureSet with empty bindings is valid and fields are default.
    #[test]
    fn test_capture_set_empty_bindings() {
        let cs = CaptureSet {
            block_span: span(10, 50),
            bindings: vec![],
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        };
        assert!(cs.bindings.is_empty());
        assert!(!cs.has_question_mark);
        assert!(!cs.has_break);
        assert!(!cs.has_continue);
        assert!(!cs.is_inside_loop);
        assert!(cs.nested_blocks.is_empty());
        assert_eq!(cs.block_span.start, 10);
        assert_eq!(cs.block_span.end, 50);
    }

    // Test 2: CapturedBinding with Write access kind round-trips.
    #[test]
    fn test_captured_binding_write_access() {
        let cb = CapturedBinding {
            binding_id: node(7),
            name: "data".to_string(),
            access_kind: CaptureAccessKind::Write,
            access_count: 3,
            access_spans: vec![span(20, 24), span(30, 34), span(40, 44)],
        };
        assert_eq!(cb.binding_id, node(7));
        assert_eq!(cb.name, "data");
        assert_eq!(cb.access_kind, CaptureAccessKind::Write);
        assert_eq!(cb.access_count, 3);
        assert_eq!(cb.access_spans.len(), 3);
        // CaptureAccessKind is non_exhaustive; can still match known variants
        assert!(matches!(cb.access_kind, CaptureAccessKind::Write));
    }

    // Test 3: StrictBoundaryViolation::ActiveAliases with multiple alias sites.
    #[test]
    fn test_strict_boundary_violation_active_aliases() {
        let violation = StrictBoundaryViolation::ActiveAliases {
            binding_id: node(1),
            alias_sites: vec![span(5, 10), span(15, 20), span(25, 30)],
        };
        match &violation {
            StrictBoundaryViolation::ActiveAliases { binding_id, alias_sites } => {
                assert_eq!(*binding_id, node(1));
                assert_eq!(alias_sites.len(), 3);
                assert_eq!(alias_sites[0].start, 5);
            }
            _ => panic!("wrong variant"),
        }
    }

    // Test 4: StrictBoundaryFact stores block_span and violation correctly.
    #[test]
    fn test_strict_boundary_fact_stores_correctly() {
        let fact = StrictBoundaryFact {
            block_span: span(100, 200),
            violation: StrictBoundaryViolation::AsyncContext {
                async_fn_span: span(0, 20),
            },
        };
        assert_eq!(fact.block_span.start, 100);
        assert_eq!(fact.block_span.end, 200);
        match &fact.violation {
            StrictBoundaryViolation::AsyncContext { async_fn_span } => {
                assert_eq!(async_fn_span.start, 0);
            }
            _ => panic!("wrong variant"),
        }
    }
}
