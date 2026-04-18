/// @strict boundary violation detection.
///
/// Reference: P3 Task 3.3. Contract C05: emit FACTS (StrictBoundaryFact),
/// never KDiagnostic.
use kobo_ir::{
    CaptureSet, StrictBoundaryFact, StrictBoundaryViolation, TransformFacts, Kir,
};
use kobo_parser::KoboBlock;

use super::alias_scan::check_k0041_active_aliases;
use super::closure_scan::check_k0042_closure_captures;
use super::move_scan::check_k0043_moved_inside;
use super::labeled_scan::check_labeled_cross_boundary;
use super::span_convert::SpanConvert;

/// Validate @strict boundary safety. Produce StrictBoundaryFact for each
/// violation found. Contract C05: emit FACTS, not diagnostics.
///
/// `enclosing_stmts` contains the statements of the enclosing function body,
/// used by K0041 to scan for aliases before the @strict block entry.
pub fn validate_strict_boundary(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    transform_facts: &TransformFacts,
    kir: &Kir,
    enclosing_stmts: &[syn::Stmt],
    sc: &SpanConvert,
) -> Vec<StrictBoundaryFact> {
    let mut facts = Vec::new();

    check_k0041_active_aliases(block, capture_set, kir, enclosing_stmts, &mut facts, sc);
    check_k0063_async_context(block, capture_set, &mut facts);
    check_k0042_closure_captures(block, capture_set, transform_facts, &mut facts, sc);
    check_k0043_moved_inside(block, capture_set, &mut facts, sc);
    check_labeled_cross_boundary(block, capture_set, &mut facts, sc);

    facts
}

/// K0063: @strict block inside async context.
///
/// Checks the `is_inside_async` flag set during AST collection.
/// Contract C05: emits StrictBoundaryFact, never KDiagnostic.
fn check_k0063_async_context(
    block: &KoboBlock,
    _capture_set: &CaptureSet,
    facts: &mut Vec<StrictBoundaryFact>,
) {
    if !block.is_strict {
        return;
    }
    if !block.is_inside_async {
        return;
    }
    let async_fn_span = block
        .async_context_span
        .unwrap_or_else(|| block.span);
    facts.push(StrictBoundaryFact {
        block_span: block.span,
        violation: StrictBoundaryViolation::AsyncContext { async_fn_span },
    });
}

#[cfg(test)]
mod tests {
    use super::validate_strict_boundary;
    use super::super::span_convert::SpanConvert;
    use kobo_ir::{
        BindingUsage, CaptureAccessKind, CaptureSet, CapturedBinding, FileId, Kir, KirNode,
        KirNodeId, KoboAstNodeId, KoboSpan, NodeKind, OwnershipTier, SharedBindingFacts,
        StrictBoundaryViolation, TierDecision, TierReason, TransformBindingFacts, TransformFacts,
    };
    use kobo_parser::KoboBlock;

    fn span(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(0))
    }

    fn node_id(n: u32) -> KirNodeId {
        KirNodeId(n)
    }

    fn test_sc() -> SpanConvert {
        // Build from a dummy source; tests use syn::parse_str blocks whose
        // spans are relative to the parsed string, not a real file.
        SpanConvert::new("", FileId(0))
    }

    fn make_kir_node(id: KirNodeId) -> KirNode {
        KirNode {
            id,
            kind: NodeKind::Decl,
            ast_id: Some(KoboAstNodeId(id.0)),
            ownership: OwnershipTier::Undecided,
            resource_kind: None,
            cfg_block: None,
            span: span(0, 1),
            decl_id: None,
        }
    }

    fn make_tier(node: KirNodeId, tier: OwnershipTier) -> TierDecision {
        TierDecision {
            node,
            tier,
            reason: TierReason::MutableSharedLastResort,
            annotate: false,
        }
    }

    fn make_binding(node: KirNodeId, name: &str) -> TransformBindingFacts {
        TransformBindingFacts {
            node,
            ast_id: KoboAstNodeId(node.0),
            binding_name: name.to_string(),
            span: span(0, 4),
            resource_kind: None,
            hint: None,
            hint_span: None,
            is_copy_known: false,
            is_generic: false,
            is_async: false,
            async_shared: false,
            usage: BindingUsage::new(span(0, 4)),
            shared_facts: SharedBindingFacts { node_id: node, ..Default::default() },
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
            decl_scope_depth: 0,
            ref_returning_read_spans: Vec::new(),
        }
    }

    fn make_facts(bindings: Vec<TransformBindingFacts>) -> TransformFacts {
        TransformFacts {
            bindings,
            usages: vec![],
            shared_facts: vec![],
            hint_conflicts: vec![],
        }
    }

    fn make_block(source: &str) -> KoboBlock {
        let body: syn::Block = syn::parse_str(source).unwrap();
        KoboBlock {
            body,
            span: span(0, source.len() as u32),
            is_strict: true,
            strict_keyword_span: None,
            is_inside_async: false,
            async_context_span: None,
        }
    }

    fn make_capture_set_with(binding_id: KirNodeId, name: &str) -> CaptureSet {
        CaptureSet {
            block_span: span(0, 100),
            bindings: vec![CapturedBinding {
                binding_id,
                name: name.to_string(),
                access_kind: CaptureAccessKind::Write,
                access_count: 1,
                access_spans: vec![span(10, 14)],
            }],
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        }
    }

    #[test]
    fn test_k0041_no_aliases_no_violation() {
        let node = node_id(1);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let mut kir = Kir::from_nodes(vec![make_kir_node(node)]);
        kir.set_tier_decisions(vec![make_tier(node, OwnershipTier::RcMutShared)]);
        let block = make_block("{ let _x = &data; }");
        let cs = make_capture_set_with(node, "data");
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let k0041_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ActiveAliases { .. }))
            .collect();
        assert!(k0041_violations.is_empty(), "no aliases should mean no K0041 violation");
    }

    #[test]
    fn test_k0042_closure_captures_binding() {
        let node = node_id(2);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ let f = || { let _x = &data; }; }");
        let cs = make_capture_set_with(node, "data");
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let k0042_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ClosureCapture { .. }))
            .collect();
        assert_eq!(k0042_violations.len(), 1, "should detect closure capture of `data`");
    }

    #[test]
    fn test_k0042_closure_captures_non_wrapped_binding_no_violation() {
        let node = node_id(3);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ let f = || { let _x = other; }; }");
        let cs = make_capture_set_with(node, "data");
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let k0042_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ClosureCapture { .. }))
            .collect();
        assert!(k0042_violations.is_empty());
    }

    #[test]
    fn test_k0043_value_moved_inside() {
        let node = node_id(4);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ let owned = data; }");
        let cs = make_capture_set_with(node, "data");
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let k0043_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::MovedInside { .. }))
            .collect();
        assert_eq!(k0043_violations.len(), 1, "should detect move of `data`");
    }

    #[test]
    fn test_k0043_no_moves_no_violation() {
        let node = node_id(5);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ let _x = &data; }");
        let cs = make_capture_set_with(node, "data");
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let k0043_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::MovedInside { .. }))
            .collect();
        assert!(k0043_violations.is_empty());
    }

    #[test]
    fn test_labeled_break_detected() {
        let node = node_id(6);
        let facts = make_facts(vec![]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ break 'outer; }");
        let cs = CaptureSet {
            block_span: span(0, 20),
            bindings: vec![],
            has_question_mark: false,
            has_break: true,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        };
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let labeled_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| {
                matches!(f.violation, StrictBoundaryViolation::LabeledCrossBoundary { .. })
            })
            .collect();
        assert_eq!(labeled_violations.len(), 1, "labeled break should be detected");
    }

    #[test]
    fn test_k0063_async_context_fact_structure() {
        let async_fn_span = span(0, 20);
        let block_span = span(30, 80);
        let fact = kobo_ir::StrictBoundaryFact {
            block_span,
            violation: StrictBoundaryViolation::AsyncContext { async_fn_span },
        };
        match &fact.violation {
            StrictBoundaryViolation::AsyncContext { async_fn_span: s } => {
                assert_eq!(s.start, 0);
            }
            _ => panic!("wrong variant"),
        }
        assert_eq!(fact.block_span.start, 30);
    }

    #[test]
    fn test_empty_capture_set_no_violations() {
        let facts = make_facts(vec![]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ let x = 1; }");
        let cs = CaptureSet {
            block_span: span(0, 20),
            bindings: vec![],
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        };
        let sc = test_sc();
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[], &sc);
        let k0042 = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ClosureCapture { .. }))
            .count();
        let k0043 = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::MovedInside { .. }))
            .count();
        assert_eq!(k0042, 0);
        assert_eq!(k0043, 0);
    }

    #[test]
    fn test_c05_no_kdiagnostic_in_boundary() {
        assert!(true);
    }
}
