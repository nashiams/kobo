/// AST visitor for capture-set analysis inside @strict blocks.
///
/// Walks the block body, records accesses to RcMutShared bindings,
/// and detects control-flow operators (?, break, continue).
use kobo_ir::{KoboSpan, TransformFacts, Kir};
use syn::visit::Visit;

use super::span_convert::SpanConvert;

/// One recorded access to an RcMutShared binding inside the block.
#[derive(Debug)]
pub(super) struct AccessRecord {
    pub name: String,
    pub binding_id: kobo_ir::KirNodeId,
    pub kind: AccessKind,
    pub span: KoboSpan,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum AccessKind {
    Read,
    Write,
}

/// Visitor that walks the block body and collects accesses to RcMutShared bindings.
pub(super) struct StrictCaptureVisitor<'a> {
    transform_facts: &'a TransformFacts,
    kir: &'a Kir,
    sc: &'a SpanConvert,
    pub accesses: Vec<AccessRecord>,
    pub has_question_mark: bool,
    pub has_break: bool,
    pub has_continue: bool,
    pub is_inside_loop: bool,
    loop_depth: u32,
}

impl<'a> StrictCaptureVisitor<'a> {
    pub fn new(transform_facts: &'a TransformFacts, kir: &'a Kir, sc: &'a SpanConvert) -> Self {
        Self {
            transform_facts,
            kir,
            sc,
            accesses: Vec::new(),
            has_question_mark: false,
            has_break: false,
            has_continue: false,
            is_inside_loop: false,
            loop_depth: 0,
        }
    }

    fn record_ident_access(&mut self, ident: &syn::Ident, kind: AccessKind) {
        let name = ident.to_string();
        let Some(binding_facts) = self
            .transform_facts
            .iter_bindings()
            .find(|b| b.binding_name == name)
        else {
            return;
        };
        let Some(decision) = self.kir.tier_decision(binding_facts.node) else {
            return;
        };
        if !decision.tier.is_shared() {
            return;
        }
        let span = self.sc.span(ident.span());
        self.accesses.push(AccessRecord {
            name,
            binding_id: binding_facts.node,
            kind,
            span,
        });
    }
}

impl<'a, 'ast> Visit<'ast> for StrictCaptureVisitor<'a> {
    fn visit_expr_try(&mut self, node: &'ast syn::ExprTry) {
        self.has_question_mark = true;
        syn::visit::visit_expr_try(self, node);
    }

    fn visit_expr_break(&mut self, node: &'ast syn::ExprBreak) {
        self.has_break = true;
        syn::visit::visit_expr_break(self, node);
    }

    fn visit_expr_continue(&mut self, node: &'ast syn::ExprContinue) {
        self.has_continue = true;
        syn::visit::visit_expr_continue(self, node);
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.loop_depth += 1;
        self.is_inside_loop = true;
        syn::visit::visit_expr_loop(self, node);
        self.loop_depth -= 1;
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.loop_depth += 1;
        self.is_inside_loop = true;
        syn::visit::visit_expr_while(self, node);
        self.loop_depth -= 1;
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.loop_depth += 1;
        self.is_inside_loop = true;
        syn::visit::visit_expr_for_loop(self, node);
        self.loop_depth -= 1;
    }

    fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
        if node.mutability.is_some() {
            if let syn::Expr::Path(path) = node.expr.as_ref() {
                if let Some(ident) = path.path.get_ident() {
                    self.record_ident_access(ident, AccessKind::Write);
                    return;
                }
            }
        }
        syn::visit::visit_expr_reference(self, node);
    }

    fn visit_expr_assign(&mut self, node: &'ast syn::ExprAssign) {
        if let syn::Expr::Path(path) = node.left.as_ref() {
            if let Some(ident) = path.path.get_ident() {
                self.record_ident_access(ident, AccessKind::Write);
                syn::visit::visit_expr(self, &node.right);
                return;
            }
        }
        if let syn::Expr::Unary(unary) = node.left.as_ref() {
            if matches!(unary.op, syn::UnOp::Deref(_)) {
                if let syn::Expr::Path(path) = unary.expr.as_ref() {
                    if let Some(ident) = path.path.get_ident() {
                        self.record_ident_access(ident, AccessKind::Write);
                        syn::visit::visit_expr(self, &node.right);
                        return;
                    }
                }
            }
        }
        syn::visit::visit_expr_assign(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if let syn::Expr::Path(path) = node.receiver.as_ref() {
            if let Some(ident) = path.path.get_ident() {
                let method_name = node.method.to_string();
                let kind = if method_name.ends_with("_mut")
                    || method_name == "borrow_mut"
                    || method_name == "push"
                    || method_name == "pop"
                    || method_name == "insert"
                    || method_name == "remove"
                    || method_name == "clear"
                    || method_name == "retain"
                {
                    AccessKind::Write
                } else {
                    AccessKind::Read
                };
                self.record_ident_access(ident, kind);
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        if let Some(ident) = node.path.get_ident() {
            self.record_ident_access(ident, AccessKind::Read);
        }
        syn::visit::visit_expr_path(self, node);
    }
}
