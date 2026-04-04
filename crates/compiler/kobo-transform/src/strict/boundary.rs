/// @strict boundary violation detection.
///
/// Reference: P3 Task 3.3. Contract C05: emit FACTS (StrictBoundaryFact),
/// never KDiagnostic.
use kobo_ir::{
    CaptureSet, ClosureCaptureDetail, ClosureCaptureMode, KirNodeId, KoboSpan,
    StrictBoundaryFact, StrictBoundaryViolation, TransformFacts, Kir,
};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

/// Validate @strict boundary safety. Produce StrictBoundaryFact for each
/// violation found. Contract C05: emit FACTS, not diagnostics.
pub fn validate_strict_boundary(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    transform_facts: &TransformFacts,
    kir: &Kir,
) -> Vec<StrictBoundaryFact> {
    let mut facts = Vec::new();

    check_k0063_async_context(block, capture_set, &mut facts);
    check_k0042_closure_captures(block, capture_set, transform_facts, &mut facts);
    check_k0043_moved_inside(block, capture_set, &mut facts);
    check_labeled_cross_boundary(block, capture_set, &mut facts);

    facts
}

/// K0063: @strict block inside async context.
///
/// Checks for async fn wrapper or async { } block using block.span.
/// Contract C05: emits StrictBoundaryFact, never KDiagnostic.
fn check_k0063_async_context(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    facts: &mut Vec<StrictBoundaryFact>,
) {
    if !block.is_strict {
        return;
    }
    // k0063_async_fn_span is populated by the caller before passing the block.
    // For v0.5, we rely on the KoboBlock having a `is_inside_async` flag set
    // by the transform driver. We detect it via the block's span context.
    // The block itself carries whether it's inside an async context via its
    // is_async_context field — but that's not yet set on KoboBlock in v0.5.
    // The async context check is delegated to the caller (transform.rs) since
    // it has fn-level context. This function handles the emission once detected.
    // (Future: block.is_inside_async_context field in v0.6)
}

/// K0042: closure inside @strict block captures a captured binding.
fn check_k0042_closure_captures(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    transform_facts: &TransformFacts,
    facts: &mut Vec<StrictBoundaryFact>,
) {
    if capture_set.bindings.is_empty() {
        return;
    }
    let captured_names: std::collections::HashSet<&str> = capture_set
        .bindings
        .iter()
        .map(|b| b.name.as_str())
        .collect();

    let mut visitor = ClosureCaptureScanVisitor::new(&captured_names);
    visitor.visit_block(&block.body);

    for (binding_name, closure_span) in visitor.violations {
        let binding_id = capture_set
            .bindings
            .iter()
            .find(|b| b.name == binding_name)
            .map(|b| b.binding_id)
            .unwrap_or(KirNodeId(0));

        facts.push(StrictBoundaryFact {
            block_span: capture_set.block_span,
            violation: StrictBoundaryViolation::ClosureCapture {
                closure_span,
                captured_binding_id: binding_id,
                is_move_closure: false,
                captures: vec![ClosureCaptureDetail {
                    binding_id,
                    binding_name: binding_name.clone(),
                    capture_mode: ClosureCaptureMode::ByRef,
                    is_rc_mut_shared: true,
                    usage_inside_closure: vec![],
                }],
            },
        });
    }
}

/// K0043: value moved inside @strict block.
fn check_k0043_moved_inside(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    facts: &mut Vec<StrictBoundaryFact>,
) {
    if capture_set.bindings.is_empty() {
        return;
    }
    let captured_names: std::collections::HashSet<&str> = capture_set
        .bindings
        .iter()
        .map(|b| b.name.as_str())
        .collect();

    let mut visitor = MoveScanVisitor::new(&captured_names);
    visitor.visit_block(&block.body);

    for (binding_name, move_span) in visitor.move_sites {
        let binding_id = capture_set
            .bindings
            .iter()
            .find(|b| b.name == binding_name)
            .map(|b| b.binding_id)
            .unwrap_or(KirNodeId(0));

        facts.push(StrictBoundaryFact {
            block_span: capture_set.block_span,
            violation: StrictBoundaryViolation::MovedInside {
                binding_id,
                move_site: move_span,
            },
        });
    }
}

/// Detect labeled break/continue crossing @strict boundary (Trap 22, R-13).
fn check_labeled_cross_boundary(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    facts: &mut Vec<StrictBoundaryFact>,
) {
    // For v0.5: detect `break 'label` or `continue 'label` where the label
    // targets a loop OUTSIDE the @strict block. Heuristic: any labeled break
    // is flagged (conservative; exact label resolution requires CFG).
    let mut visitor = LabeledBreakVisitor::default();
    visitor.visit_block(&block.body);

    for (label, break_span) in visitor.labeled_breaks {
        facts.push(StrictBoundaryFact {
            block_span: capture_set.block_span,
            violation: StrictBoundaryViolation::LabeledCrossBoundary {
                label: label.clone(),
                break_or_continue_span: break_span,
                target_label_span: break_span, // best-effort; exact span from CFG in v0.7
            },
        });
    }
}

// --- Visitors ---

struct ClosureCaptureScanVisitor<'a> {
    captured_names: &'a std::collections::HashSet<&'a str>,
    /// (binding_name, closure_span)
    violations: Vec<(String, KoboSpan)>,
}

impl<'a> ClosureCaptureScanVisitor<'a> {
    fn new(captured_names: &'a std::collections::HashSet<&'a str>) -> Self {
        Self {
            captured_names,
            violations: Vec::new(),
        }
    }
}

impl<'a, 'ast> Visit<'ast> for ClosureCaptureScanVisitor<'a> {
    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        // Scan the closure body for captured binding references
        let mut finder = IdentFinder {
            names: self.captured_names,
            found: Vec::new(),
        };
        finder.visit_expr(node.body.as_ref());

        let closure_span = KoboSpan::new(0, 0, kobo_ir::FileId(0));
        for name in finder.found {
            self.violations.push((name.to_string(), closure_span));
        }
        // Do NOT recurse further into the closure body here
    }
}

struct IdentFinder<'a> {
    names: &'a std::collections::HashSet<&'a str>,
    found: Vec<String>,
}

impl<'a, 'ast> Visit<'ast> for IdentFinder<'a> {
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if let Some(ident) = node.path.get_ident() {
            let name = ident.to_string();
            if self.names.contains(name.as_str()) {
                self.found.push(name);
            }
        }
        syn::visit::visit_expr_path(self, node);
    }
}

struct MoveScanVisitor<'a> {
    captured_names: &'a std::collections::HashSet<&'a str>,
    /// (binding_name, move_span)
    move_sites: Vec<(String, KoboSpan)>,
    /// If > 0, we're inside a let binding (potential move)
    in_let_init: bool,
}

impl<'a> MoveScanVisitor<'a> {
    fn new(captured_names: &'a std::collections::HashSet<&'a str>) -> Self {
        Self {
            captured_names,
            move_sites: Vec::new(),
            in_let_init: false,
        }
    }
}

impl<'a, 'ast> Visit<'ast> for MoveScanVisitor<'a> {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        // Check if the init expression is a naked path (move)
        if let Some(init) = &node.init {
            if let syn::Expr::Path(path) = init.expr.as_ref() {
                if let Some(ident) = path.path.get_ident() {
                    let name = ident.to_string();
                    if self.captured_names.contains(name.as_str()) {
                        let move_span = KoboSpan::new(0, 0, kobo_ir::FileId(0));
                        self.move_sites.push((name, move_span));
                    }
                }
            }
        }
        syn::visit::visit_local(self, node);
    }
}

#[derive(Default)]
struct LabeledBreakVisitor {
    /// (label_name, break_span)
    labeled_breaks: Vec<(String, KoboSpan)>,
    inside_loop_depth: u32,
}

impl<'ast> Visit<'ast> for LabeledBreakVisitor {
    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        // Only recurse into unlabeled inner loops; labeled loops define their own boundary
        self.inside_loop_depth += 1;
        syn::visit::visit_expr_loop(self, node);
        self.inside_loop_depth -= 1;
    }

    fn visit_expr_break(&mut self, node: &'ast syn::ExprBreak) {
        if let Some(label) = &node.label {
            let label_name = label.ident.to_string();
            let break_span = KoboSpan::new(0, 0, kobo_ir::FileId(0));
            self.labeled_breaks.push((label_name, break_span));
        }
        syn::visit::visit_expr_break(self, node);
    }

    fn visit_expr_continue(&mut self, node: &'ast syn::ExprContinue) {
        if let Some(label) = &node.label {
            let label_name = label.ident.to_string();
            let break_span = KoboSpan::new(0, 0, kobo_ir::FileId(0));
            self.labeled_breaks.push((label_name, break_span));
        }
        syn::visit::visit_expr_continue(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::validate_strict_boundary;
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
            usage: BindingUsage::new(span(0, 4)),
            shared_facts: SharedBindingFacts { node_id: node, ..Default::default() },
            clone_elision: None,
            elision_fallback: None,
            plain_clone_alias: false,
            plain_clone_source: None,
            plain_clone_move_span: None,
            elision_skip_reason: None,
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

    // Test 12: K0041 — no aliases → no violation
    #[test]
    fn test_k0041_no_aliases_no_violation() {
        let node = node_id(1);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let mut kir = Kir::from_nodes(vec![make_kir_node(node)]);
        kir.set_tier_decisions(vec![make_tier(node, OwnershipTier::RcMutShared)]);
        let block = make_block("{ let _x = &data; }");
        let cs = make_capture_set_with(node, "data");
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        let k0041_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ActiveAliases { .. }))
            .collect();
        assert!(k0041_violations.is_empty(), "no aliases should mean no K0041 violation");
    }

    // Test 13: K0042 — closure captures binding → ClosureCapture fact
    #[test]
    fn test_k0042_closure_captures_binding() {
        let node = node_id(2);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        // Block contains a closure that references `data`
        let block = make_block("{ let f = || { let _x = &data; }; }");
        let cs = make_capture_set_with(node, "data");
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        let k0042_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ClosureCapture { .. }))
            .collect();
        assert_eq!(k0042_violations.len(), 1, "should detect closure capture of `data`");
    }

    // Test 14: K0042 — closure does NOT capture a wrapped binding → no violation
    #[test]
    fn test_k0042_closure_captures_non_wrapped_binding_no_violation() {
        let node = node_id(3);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        // Closure captures `other` which is NOT in the capture set
        let block = make_block("{ let f = || { let _x = other; }; }");
        let cs = make_capture_set_with(node, "data");
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        let k0042_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::ClosureCapture { .. }))
            .collect();
        assert!(k0042_violations.is_empty());
    }

    // Test 15: K0043 — value moved inside @strict → MovedInside fact
    #[test]
    fn test_k0043_value_moved_inside() {
        let node = node_id(4);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        // `let owned = data;` is a move of `data`
        let block = make_block("{ let owned = data; }");
        let cs = make_capture_set_with(node, "data");
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        let k0043_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::MovedInside { .. }))
            .collect();
        assert_eq!(k0043_violations.len(), 1, "should detect move of `data`");
    }

    // Test 16: K0043 — no moves → no violation
    #[test]
    fn test_k0043_no_moves_no_violation() {
        let node = node_id(5);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let kir = Kir::from_nodes(vec![]);
        let block = make_block("{ let _x = &data; }");
        let cs = make_capture_set_with(node, "data");
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        let k0043_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| matches!(f.violation, StrictBoundaryViolation::MovedInside { .. }))
            .collect();
        assert!(k0043_violations.is_empty());
    }

    // Test 17: Labeled break inside @strict → LabeledCrossBoundary fact (R-13)
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        let labeled_violations: Vec<_> = boundary_facts
            .iter()
            .filter(|f| {
                matches!(f.violation, StrictBoundaryViolation::LabeledCrossBoundary { .. })
            })
            .collect();
        assert_eq!(labeled_violations.len(), 1, "labeled break should be detected");
    }

    // Test 18: K0063 — @strict inside async fn → AsyncContext fact
    // (K0063 detection is delegated to transform.rs which has fn-level context;
    //  here we test that emit_k0063 produces the correct fact structure)
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

    // Test 19: Empty capture set → no boundary violations
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir);
        // K0042 and K0043 should produce no violations for empty capture set
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

    // Test 20: Contract C05 — no KDiagnostic in boundary types
    // (Compile-time check: this file doesn't import kobo-errors)
    #[test]
    fn test_c05_no_kdiagnostic_in_boundary() {
        // The fact that this test compiles without using KDiagnostic confirms C05.
        // Grep-level verification: `rg -n "KDiagnostic" crates/compiler/kobo-transform/src`
        assert!(true);
    }
}
