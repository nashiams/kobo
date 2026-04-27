/// K0043: value moved inside @strict block.
///
/// Detects move patterns: let-binding, function call by-value, method call
/// by-value, match scrutinee, return expression.
///
/// Contract C05: produces facts, NOT KDiagnostic.
use kobo_ir::{CaptureSet, KirNodeId, KoboSpan, StrictBoundaryFact, StrictBoundaryViolation};
use kobo_parser::KoboBlock;
use syn::visit::Visit;

use super::span_convert::SpanConvert;

pub(super) fn check_k0043_moved_inside(
    block: &KoboBlock,
    capture_set: &CaptureSet,
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

    let mut visitor = MoveScanVisitor::new(&captured_names, sc);
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

// --- MoveScanVisitor ---

struct MoveScanVisitor<'a> {
    captured_names: &'a std::collections::HashSet<&'a str>,
    move_sites: Vec<(String, KoboSpan)>,
    sc: &'a SpanConvert,
}

impl<'a> MoveScanVisitor<'a> {
    fn new(captured_names: &'a std::collections::HashSet<&'a str>, sc: &'a SpanConvert) -> Self {
        Self {
            captured_names,
            move_sites: Vec::new(),
            sc,
        }
    }

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

    fn record_move(&mut self, name: String, node_span: proc_macro2::Span) {
        let move_span = self.sc.span(node_span);
        self.move_sites.push((name, move_span));
    }
}

impl<'a, 'ast> Visit<'ast> for MoveScanVisitor<'a> {
    fn visit_local(&mut self, node: &'ast syn::Local) {
        if let Some(init) = &node.init {
            if let Some(name) = self.is_captured_ident(&init.expr) {
                use syn::spanned::Spanned;
                self.record_move(name, node.span());
            }
        }
        syn::visit::visit_local(self, node);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        for arg in &call.args {
            if let Some(name) = self.is_captured_ident(arg) {
                use syn::spanned::Spanned;
                self.record_move(name, call.span());
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, method: &'ast syn::ExprMethodCall) {
        for arg in &method.args {
            if let Some(name) = self.is_captured_ident(arg) {
                use syn::spanned::Spanned;
                self.record_move(name, method.span());
            }
        }
        syn::visit::visit_expr_method_call(self, method);
    }

    fn visit_expr_match(&mut self, expr_match: &'ast syn::ExprMatch) {
        if let Some(name) = self.is_captured_ident(&expr_match.expr) {
            use syn::spanned::Spanned;
            self.record_move(name, expr_match.span());
        }
        syn::visit::visit_expr_match(self, expr_match);
    }

    fn visit_expr_return(&mut self, expr_return: &'ast syn::ExprReturn) {
        if let Some(ref expr) = expr_return.expr {
            if let Some(name) = self.is_captured_ident(expr) {
                use syn::spanned::Spanned;
                self.record_move(name, expr_return.span());
            }
        }
        syn::visit::visit_expr_return(self, expr_return);
    }
}
