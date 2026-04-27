/// K0042: closure inside @strict block captures a captured binding.
///
/// Contract C05: produces facts, NOT KDiagnostic.
use kobo_ir::{
    CaptureSet, ClosureCaptureDetail, ClosureCaptureMode, KirNodeId, KoboSpan, StrictBoundaryFact,
    StrictBoundaryViolation, TransformFacts,
};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

use super::span_convert::SpanConvert;

pub(super) fn check_k0042_closure_captures(
    block: &KoboBlock,
    capture_set: &CaptureSet,
    _transform_facts: &TransformFacts,
    facts: &mut Vec<StrictBoundaryFact>,
    sc: &SpanConvert,
) {
    if capture_set.bindings.is_empty() {
        return;
    }
    let captured_names: std::collections::HashSet<&str> = capture_set
        .bindings
        .iter()
        .map(|b| b.name.as_str())
        .collect();

    let mut visitor = ClosureCaptureScanVisitor::new(&captured_names, sc);
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

// --- ClosureCaptureScanVisitor ---

struct ClosureCaptureScanVisitor<'a> {
    captured_names: &'a std::collections::HashSet<&'a str>,
    violations: Vec<(String, KoboSpan)>,
    sc: &'a SpanConvert,
}

impl<'a> ClosureCaptureScanVisitor<'a> {
    fn new(captured_names: &'a std::collections::HashSet<&'a str>, sc: &'a SpanConvert) -> Self {
        Self {
            captured_names,
            violations: Vec::new(),
            sc,
        }
    }
}

impl<'a, 'ast> Visit<'ast> for ClosureCaptureScanVisitor<'a> {
    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        let mut finder = IdentFinder {
            names: self.captured_names,
            found: Vec::new(),
        };
        finder.visit_expr(node.body.as_ref());

        use syn::spanned::Spanned;
        let closure_span = self.sc.span(node.span());
        for name in finder.found {
            self.violations.push((name.to_string(), closure_span));
        }
    }
}

// --- IdentFinder ---

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
