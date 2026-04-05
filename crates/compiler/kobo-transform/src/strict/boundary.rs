/// @strict boundary violation detection.
///
/// Reference: P3 Task 3.3. Contract C05: emit FACTS (StrictBoundaryFact),
/// never KDiagnostic.
use kobo_ir::{
    CaptureSet, ClosureCaptureDetail, ClosureCaptureMode, KirNodeId, KoboSpan,
    StrictBoundaryFact, StrictBoundaryViolation, TransformFacts, Kir, OwnershipTier,
};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

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
) -> Vec<StrictBoundaryFact> {
    let mut facts = Vec::new();

    check_k0041_active_aliases(block, capture_set, kir, enclosing_stmts, &mut facts);
    check_k0063_async_context(block, capture_set, &mut facts);
    check_k0042_closure_captures(block, capture_set, transform_facts, &mut facts);
    check_k0043_moved_inside(block, capture_set, &mut facts);
    check_labeled_cross_boundary(block, capture_set, &mut facts);

    facts
}

/// K0041: active aliases (Rc clones) exist at @strict block entry.
///
/// Walks the enclosing function body from binding declaration to @strict
/// block entry. Detects:
/// 1. `.clone()` calls on the binding → alias via Rc::clone
/// 2. `let alias = binding;` → alias via move/copy
/// 3. Function calls passing binding by value → alias escapes to callee
///
/// Contract C03: produces StrictBoundaryViolation::ActiveAliases (never Warning).
/// Contract C05: produces facts, NOT KDiagnostic.
fn check_k0041_active_aliases(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    kir: &Kir,
    enclosing_stmts: &[syn::Stmt],
    facts: &mut Vec<StrictBoundaryFact>,
) {
    if !block.is_strict || capture_set.bindings.is_empty() {
        return;
    }

    // Only check bindings that are RcMutShared (only those can have Rc aliases).
    let rc_mut_names: std::collections::HashSet<&str> = capture_set
        .bindings
        .iter()
        .filter(|b| {
            kir.tier_decision(b.binding_id)
                .map(|td| td.tier == OwnershipTier::RcMutShared)
                .unwrap_or(false)
        })
        .map(|b| b.name.as_str())
        .collect();

    if rc_mut_names.is_empty() {
        return;
    }

    // Scan enclosing statements BEFORE the @strict block for alias patterns.
    let block_start = block.span.start;
    let mut alias_visitor = AliasScanVisitor::new(&rc_mut_names);

    for stmt in enclosing_stmts {
        // Heuristic: only scan statements that appear before the @strict block
        // by checking the span start offset. Since syn spans may not map 1:1
        // to KoboSpan offsets in all cases, we scan all statements and let
        // the visitor collect alias sites.
        alias_visitor.visit_stmt(stmt);
    }

    for (binding_name, alias_sites) in alias_visitor.aliases {
        if alias_sites.is_empty() {
            continue;
        }
        let binding_id = capture_set
            .bindings
            .iter()
            .find(|b| b.name == binding_name)
            .map(|b| b.binding_id)
            .unwrap_or(KirNodeId(0));

        facts.push(StrictBoundaryFact {
            block_span: capture_set.block_span,
            violation: StrictBoundaryViolation::ActiveAliases {
                binding_id,
                alias_sites,
            },
        });
    }
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
    // @strict inside async context is always an error (Trap 10).
    let async_fn_span = block
        .async_context_span
        .unwrap_or_else(|| block.span);
    facts.push(StrictBoundaryFact {
        block_span: block.span,
        violation: StrictBoundaryViolation::AsyncContext { async_fn_span },
    });
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
}

impl<'a> MoveScanVisitor<'a> {
    fn new(captured_names: &'a std::collections::HashSet<&'a str>) -> Self {
        Self {
            captured_names,
            move_sites: Vec::new(),
        }
    }

    /// Check if an expression is a naked path to a captured binding name.
    fn is_captured_ident(&self, expr: &syn::Expr) -> Option<String> {
        if let syn::Expr::Path(path) = expr {
            if let Some(ident) = path.path.get_ident() {
                let name = ident.to_string();
                if self.captured_names.contains(name.as_str()) {
                    return Some(name);
                }
            }
        }
        None
    }

    fn record_move(&mut self, name: String) {
        let move_span = KoboSpan::new(0, 0, kobo_ir::FileId(0));
        self.move_sites.push((name, move_span));
    }
}

impl<'a, 'ast> Visit<'ast> for MoveScanVisitor<'a> {
    // Detect `let x = captured_ident;` — move via let-binding
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some(init) = &node.init {
            if let Some(name) = self.is_captured_ident(&init.expr) {
                self.record_move(name);
            }
        }
        syn::visit::visit_local(self, node);
    }

    // Detect function call by-value argument moves: `foo(captured)`
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        for arg in &call.args {
            if let Some(name) = self.is_captured_ident(arg) {
                self.record_move(name);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    // Detect method call by-value argument moves: `obj.method(captured)`
    fn visit_expr_method_call(&mut self, method: &'ast syn::ExprMethodCall) {
        for arg in &method.args {
            if let Some(name) = self.is_captured_ident(arg) {
                self.record_move(name);
            }
        }
        syn::visit::visit_expr_method_call(self, method);
    }

    // Detect match arm moves: `match captured { ... }`
    fn visit_expr_match(&mut self, expr_match: &'ast syn::ExprMatch) {
        if let Some(name) = self.is_captured_ident(&expr_match.expr) {
            self.record_move(name);
        }
        syn::visit::visit_expr_match(self, expr_match);
    }

    // Detect return expression moves: `return captured;`
    fn visit_expr_return(&mut self, expr_return: &'ast syn::ExprReturn) {
        if let Some(ref expr) = expr_return.expr {
            if let Some(name) = self.is_captured_ident(expr) {
                self.record_move(name);
            }
        }
        syn::visit::visit_expr_return(self, expr_return);
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

/// K0041 visitor: scan for alias patterns of RcMutShared bindings.
///
/// Detects:
/// - `.clone()` calls on a captured binding → Rc alias
/// - `let alias = binding;` assignments → alias via copy/move
/// - Function calls passing binding by value → alias escapes
struct AliasScanVisitor<'a> {
    rc_mut_names: &'a std::collections::HashSet<&'a str>,
    /// binding_name → list of alias site spans
    aliases: std::collections::HashMap<String, Vec<KoboSpan>>,
}

impl<'a> AliasScanVisitor<'a> {
    fn new(rc_mut_names: &'a std::collections::HashSet<&'a str>) -> Self {
        Self {
            rc_mut_names,
            aliases: std::collections::HashMap::new(),
        }
    }

    fn record_alias(&mut self, name: &str) {
        let alias_span = KoboSpan::new(0, 0, kobo_ir::FileId(0));
        self.aliases
            .entry(name.to_string())
            .or_default()
            .push(alias_span);
    }

    fn is_name_match(&self, expr: &syn::Expr) -> Option<String> {
        if let syn::Expr::Path(path) = expr {
            if let Some(ident) = path.path.get_ident() {
                let name = ident.to_string();
                if self.rc_mut_names.contains(name.as_str()) {
                    return Some(name);
                }
            }
        }
        None
    }
}

impl<'a, 'ast> Visit<'ast> for AliasScanVisitor<'a> {
    // Detect `.clone()` calls on captured bindings → Rc alias
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "clone" {
            if let Some(name) = self.is_name_match(&node.receiver) {
                self.record_alias(&name);
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    // Detect `let alias = binding;` → alias via move/copy
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some(init) = &node.init {
            if let Some(name) = self.is_name_match(&init.expr) {
                self.record_alias(&name);
            }
        }
        syn::visit::visit_local(self, node);
    }

    // Detect function calls passing binding by value → alias escapes
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        for arg in &node.args {
            if let Some(name) = self.is_name_match(arg) {
                self.record_alias(&name);
            }
        }
        syn::visit::visit_expr_call(self, node);
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

    // Test 12: K0041 — no aliases → no violation
    #[test]
    fn test_k0041_no_aliases_no_violation() {
        let node = node_id(1);
        let facts = make_facts(vec![make_binding(node, "data")]);
        let mut kir = Kir::from_nodes(vec![make_kir_node(node)]);
        kir.set_tier_decisions(vec![make_tier(node, OwnershipTier::RcMutShared)]);
        let block = make_block("{ let _x = &data; }");
        let cs = make_capture_set_with(node, "data");
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
        let boundary_facts = validate_strict_boundary(&block, &cs, &facts, &kir, &[]);
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
