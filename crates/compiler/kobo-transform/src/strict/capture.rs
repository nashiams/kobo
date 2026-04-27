/// Capture-set analysis for @strict blocks.
///
/// Reference: P3 Task 3.1. Contract C01: exactly ONE definition of
/// `analyze_strict_capture_set` in the entire codebase.
use kobo_ir::{CaptureAccessKind, CaptureSet, CapturedBinding, Kir, KoboSpan, TransformFacts};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

use super::capture_visitor::{AccessKind, AccessRecord, StrictCaptureVisitor};
use super::span_convert::SpanConvert;

/// THE single source of truth for capture set computation.
///
/// Contract C01: exactly one definition of this function in the entire codebase.
pub fn analyze_strict_capture_set(
    block: &KoboBlock,
    transform_facts: &TransformFacts,
    kir: &Kir,
    sc: &SpanConvert,
) -> CaptureSet {
    let mut visitor = StrictCaptureVisitor::new(transform_facts, kir, sc);
    visitor.visit_block(&block.body);

    let bindings = build_captured_bindings(visitor.accesses);

    CaptureSet {
        block_span: block.span,
        bindings,
        has_question_mark: visitor.has_question_mark,
        has_break: visitor.has_break,
        has_continue: visitor.has_continue,
        is_inside_loop: visitor.is_inside_loop,
        nested_blocks: Vec::new(),
    }
}

/// Merge access records into CapturedBinding entries.
/// Per R-12: max(Read, Write) = Write for the same binding.
fn build_captured_bindings(accesses: Vec<AccessRecord>) -> Vec<CapturedBinding> {
    use std::collections::HashMap;

    let mut by_id: HashMap<kobo_ir::KirNodeId, (String, AccessKind, Vec<KoboSpan>)> =
        HashMap::new();

    for access in accesses {
        let entry = by_id
            .entry(access.binding_id)
            .or_insert_with(|| (access.name.clone(), AccessKind::Read, Vec::new()));
        if access.kind == AccessKind::Write {
            entry.1 = AccessKind::Write;
        }
        entry.2.push(access.span);
    }

    let mut result: Vec<CapturedBinding> = by_id
        .into_iter()
        .map(|(node_id, (name, kind, spans))| {
            let access_count = spans.len();
            CapturedBinding {
                binding_id: node_id,
                name,
                access_kind: match kind {
                    AccessKind::Write => CaptureAccessKind::Write,
                    AccessKind::Read => CaptureAccessKind::Read,
                },
                access_count,
                access_spans: spans,
            }
        })
        .collect();

    result.sort_by_key(|b| b.binding_id);
    result
}

#[cfg(test)]
mod tests {
    use super::super::span_convert::SpanConvert;
    use super::analyze_strict_capture_set;
    use kobo_ir::{
        BindingUsage, CaptureAccessKind, FileId, Kir, KirNode, KirNodeId, KoboAstNodeId, KoboSpan,
        NodeKind, OwnershipTier, SharedBindingFacts, TierDecision, TierReason,
        TransformBindingFacts, TransformFacts,
    };
    use kobo_parser::KoboBlock;

    fn test_sc() -> SpanConvert {
        SpanConvert::new("", FileId(0))
    }

    fn span(start: u32, end: u32) -> KoboSpan {
        KoboSpan::new(start, end, FileId(0))
    }

    fn node_id(n: u32) -> KirNodeId {
        KirNodeId(n)
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
            shared_facts: SharedBindingFacts {
                node_id: node,
                ..Default::default()
            },
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

    fn make_kir_with_tier(binding_node: KirNodeId, tier: OwnershipTier) -> Kir {
        let mut kir = Kir::from_nodes(vec![make_kir_node(binding_node)]);
        kir.set_tier_decisions(vec![make_tier(binding_node, tier)]);
        kir
    }

    #[test]
    fn test_single_mutable_binding_captured_as_write() {
        let node = node_id(1);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = make_kir_with_tier(node, OwnershipTier::RcMutShared);
        let block = make_block("{ let _x = &mut data; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert_eq!(cs.bindings.len(), 1);
        assert_eq!(cs.bindings[0].name, "data");
        assert_eq!(cs.bindings[0].access_kind, CaptureAccessKind::Write);
    }

    #[test]
    fn test_single_read_only_binding_captured_as_read() {
        let node = node_id(2);
        let facts = make_facts(vec![make_binding(node, "value")]);
        let kir = make_kir_with_tier(node, OwnershipTier::RcMutShared);
        let block = make_block("{ let _x = &value; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert_eq!(cs.bindings.len(), 1);
        assert_eq!(cs.bindings[0].access_kind, CaptureAccessKind::Read);
    }

    #[test]
    fn test_two_bindings_one_read_one_write() {
        let n1 = node_id(1);
        let n2 = node_id(2);
        let facts = make_facts(vec![make_binding(n1, "reader"), make_binding(n2, "writer")]);
        let mut kir = Kir::from_nodes(vec![make_kir_node(n1), make_kir_node(n2)]);
        kir.set_tier_decisions(vec![
            make_tier(n1, OwnershipTier::RcMutShared),
            make_tier(n2, OwnershipTier::RcMutShared),
        ]);
        let block = make_block("{ let _a = &reader; let _b = &mut writer; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert_eq!(cs.bindings.len(), 2);
        let r = cs.bindings.iter().find(|b| b.name == "reader").unwrap();
        let w = cs.bindings.iter().find(|b| b.name == "writer").unwrap();
        assert_eq!(r.access_kind, CaptureAccessKind::Read);
        assert_eq!(w.access_kind, CaptureAccessKind::Write);
    }

    #[test]
    fn test_plain_owned_binding_not_captured() {
        let node = node_id(3);
        let facts = make_facts(vec![make_binding(node, "plain")]);
        let kir = make_kir_with_tier(node, OwnershipTier::PlainOwned);
        let block = make_block("{ let _x = &mut plain; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert!(cs.bindings.is_empty(), "PlainOwned should not be captured");
    }

    #[test]
    fn test_scoped_binding_not_captured() {
        let node = node_id(4);
        let facts = make_facts(vec![make_binding(node, "handle")]);
        let kir = make_kir_with_tier(node, OwnershipTier::Scoped);
        let block = make_block("{ let _x = &handle; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert!(
            cs.bindings.is_empty(),
            "ScopedHandle should not be captured"
        );
    }

    #[test]
    fn test_empty_block_empty_capture_set() {
        let facts = make_facts(vec![]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{}");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert!(cs.bindings.is_empty());
        assert!(!cs.has_question_mark);
        assert!(!cs.has_break);
        assert!(!cs.has_continue);
    }

    #[test]
    fn test_question_mark_sets_flag() {
        let node = node_id(5);
        let facts = make_facts(vec![make_binding(node, "res")]);
        let kir = make_kir_with_tier(node, OwnershipTier::RcMutShared);
        let block = make_block("{ let _x = some_fn()?; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert!(cs.has_question_mark, "has_question_mark should be true");
    }

    #[test]
    fn test_break_sets_flag() {
        let facts = make_facts(vec![]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ loop { break; } }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert!(cs.has_break, "has_break should be true");
    }

    #[test]
    fn test_continue_sets_flag() {
        let facts = make_facts(vec![]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ loop { continue; } }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert!(cs.has_continue, "has_continue should be true");
    }

    #[test]
    fn test_nested_strict_outer_captures_only_outer() {
        let n1 = node_id(1);
        let facts = make_facts(vec![make_binding(n1, "data")]);
        let kir = make_kir_with_tier(n1, OwnershipTier::RcMutShared);
        let block = make_block("{ let _x = &data; }");
        let sc = test_sc();
        let outer_cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert_eq!(outer_cs.bindings.len(), 1);
    }

    #[test]
    fn test_write_escalates_read_in_merge() {
        use super::super::nesting::flatten_nested_strict;
        use kobo_ir::{CaptureSet, CapturedBinding};

        let n1 = node_id(1);
        let mut outer = CaptureSet {
            block_span: span(0, 100),
            bindings: vec![CapturedBinding {
                binding_id: n1,
                name: "data".to_string(),
                access_kind: CaptureAccessKind::Read,
                access_count: 1,
                access_spans: vec![span(10, 14)],
            }],
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        };
        let inner = CaptureSet {
            block_span: span(20, 80),
            bindings: vec![CapturedBinding {
                binding_id: n1,
                name: "data".to_string(),
                access_kind: CaptureAccessKind::Write,
                access_count: 1,
                access_spans: vec![span(30, 34)],
            }],
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            nested_blocks: vec![],
        };
        flatten_nested_strict(&mut outer, vec![inner]);
        let binding = outer.bindings.iter().find(|b| b.binding_id == n1).unwrap();
        assert_eq!(
            binding.access_kind,
            CaptureAccessKind::Write,
            "Write should escalate Read after merge"
        );
    }

    #[test]
    fn test_rc_shared_binding_captured() {
        let node = node_id(10);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = make_kir_with_tier(node, OwnershipTier::RcShared);
        let block = make_block("{ let _x = &data; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert_eq!(cs.bindings.len(), 1, "RcShared should be captured");
        assert_eq!(cs.bindings[0].name, "data");
    }

    #[test]
    fn test_c05_no_kdiagnostic_in_capture() {
        let node = node_id(10);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = make_kir_with_tier(node, OwnershipTier::RcShared);
        let block = make_block("{ let _x = &data; }");
        let sc = test_sc();
        let cs = analyze_strict_capture_set(&block, &facts, &kir, &sc);
        assert_eq!(cs.bindings[0].name, "data");
    }
}
